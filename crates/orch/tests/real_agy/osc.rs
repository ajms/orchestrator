use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::Path;
use std::time::{Duration, Instant};

use orch_holder::{FromHolder, ToHolder};

use crate::agy::{RealAgy, STARTUP, is_trust_screen};

const QUERY: &[u8] = b"\x1b]11;?";
const LIGHT_BACKGROUND: &[u8] = b"\x1b]11;rgb:ffff/ffff/ffff\x1b\\";

pub struct Probe {
    pub asked: Option<(Duration, Duration)>,
    pub trusted_at: Duration,
    pub colours: BTreeSet<String>,
}

pub fn osc_11_stall(chunks: &[(Duration, Vec<u8>)], end: Duration) -> Option<(Duration, Duration)> {
    let bytes: Vec<u8> = chunks.iter().flat_map(|(_, chunk)| chunk.clone()).collect();
    let start = bytes
        .windows(QUERY.len())
        .position(|window| window == QUERY)?;
    let after = start + QUERY.len();
    let query_end = after
        + if bytes.get(after) == Some(&0x07) {
            1
        } else {
            2
        };
    let mut offset = 0;
    let mut asked_at = None;
    for (at, chunk) in chunks {
        let chunk_end = offset + chunk.len();
        match asked_at {
            Some(asked) => return Some((asked, *at - asked)),
            None if query_end < chunk_end => return Some((*at, Duration::ZERO)),
            None if query_end == chunk_end => asked_at = Some(*at),
            None => {}
        }
        offset = chunk_end;
    }
    asked_at.map(|asked| (asked, end - asked))
}

fn colours(screen: &vt100::Screen) -> BTreeSet<String> {
    let (rows, cols) = screen.size();
    (0..rows)
        .flat_map(|row| (0..cols).map(move |col| (row, col)))
        .filter_map(|(row, col)| screen.cell(row, col))
        .filter(|cell| cell.has_contents())
        .map(|cell| format!("{:?}/{:?}", cell.fgcolor(), cell.bgcolor()))
        .collect()
}

pub async fn probe(agy: &RealAgy, session: &str, cwd: &Path, answer: bool) -> Probe {
    let argv: Vec<OsString> = vec![
        "sh".into(),
        "-c".into(),
        "sleep 1; exec \"$0\"".into(),
        agy.agy.clone().into(),
    ];
    let started = Instant::now();
    let mut held = agy.hold(session, cwd, &argv).await;
    held.client.send(&ToHolder::Subscribe).await.unwrap();
    let mut chunks = Vec::new();
    let mut screen = vt100::Parser::new(24, 80, 0);
    let mut answered = false;
    while !is_trust_screen(&screen.screen().contents().to_lowercase()) {
        let remaining = STARTUP.saturating_sub(started.elapsed());
        let message = tokio::time::timeout(remaining, held.client.recv()).await;
        let Ok(Ok(Some(message))) = message else {
            panic!(
                "agy never showed its trust screen in the Holder:\n{}",
                screen.screen().contents()
            );
        };
        match message {
            FromHolder::Screen(snapshot) => screen = snapshot.restore(0),
            FromHolder::Output { bytes } => {
                screen.process(&bytes);
                chunks.push((started.elapsed(), bytes));
            }
            _ => {}
        }
        if answer && !answered && osc_11_stall(&chunks, started.elapsed()).is_some() {
            let reply = ToHolder::Input {
                bytes: LIGHT_BACKGROUND.to_vec(),
            };
            held.client.send(&reply).await.unwrap();
            answered = true;
        }
    }
    let trusted_at = started.elapsed();
    Probe {
        asked: osc_11_stall(&chunks, trusted_at),
        trusted_at,
        colours: colours(screen.screen()),
    }
}

pub fn finding(unanswered: &Probe, answered: &Probe) -> String {
    let asked = match unanswered.asked {
        Some((at, silence)) => format!(
            "agy sent an OSC 11 query {at:?} after spawn (1 s of it is the wrapper's sleep); the next output followed after {silence:?}"
        ),
        None => "agy sent no OSC 11 query before its trust screen".into(),
    };
    let theme = match unanswered.colours == answered.colours {
        true => "the trust screen used the same colours whether or not the query was answered with a light background".into(),
        false => format!(
            "the trust screen's colours differ: unanswered {:?}, answered with a light background {:?}",
            unanswered.colours, answered.colours
        ),
    };
    format!(
        "OSC 11 finding (agy in orch's Holder): {asked}. Trust screen after {:?} unanswered, {:?} answered. {theme}.\n",
        unanswered.trusted_at, answered.trusted_at
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(ms: u64) -> Duration {
        Duration::from_millis(ms)
    }

    #[test]
    fn the_silence_after_the_query_runs_to_the_next_output() {
        let chunks = [
            (at(10), b"hello".to_vec()),
            (at(20), b"\x1b]11;?\x07".to_vec()),
            (at(520), b"screen".to_vec()),
        ];

        assert_eq!(osc_11_stall(&chunks, at(900)), Some((at(20), at(500))));
    }

    #[test]
    fn output_right_after_the_query_in_the_same_chunk_is_no_silence() {
        let chunks = [(at(20), b"\x1b]11;?\x1b\\more".to_vec())];

        assert_eq!(
            osc_11_stall(&chunks, at(900)),
            Some((at(20), Duration::ZERO))
        );
    }

    #[test]
    fn a_query_split_across_chunks_counts_from_the_chunk_that_ends_it() {
        let chunks = [
            (at(20), b"\x1b]1".to_vec()),
            (at(30), b"1;?\x07".to_vec()),
            (at(2030), b"x".to_vec()),
        ];

        assert_eq!(osc_11_stall(&chunks, at(3000)), Some((at(30), at(2000))));
    }

    #[test]
    fn a_query_never_followed_by_output_is_silent_until_the_end() {
        let chunks = [(at(20), b"\x1b]11;?\x07".to_vec())];

        assert_eq!(osc_11_stall(&chunks, at(5020)), Some((at(20), at(5000))));
    }

    #[test]
    fn no_query_means_no_finding() {
        assert_eq!(osc_11_stall(&[(at(1), b"plain".to_vec())], at(9)), None);
    }
}
