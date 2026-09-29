use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, TrySendError};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use orch_config::{ConfigLoader, Notifications};
use orch_core::SessionId;
use orch_notify::{
    AttentionContext, AttentionEvent, ClientId, ClientView, DesktopAction, FreedesktopSink,
    NotificationKey, NotificationPolicy, NotificationSink, RecordingSink,
};
use orch_protocol::FromDaemon;
use serde_json::json;

use crate::NotificationTarget;
use crate::outbox::Outbox;

const QUEUE_CAPACITY: usize = 1024;
const CONFIG_TTL: Duration = Duration::from_secs(1);
const CLICK_POLL: Duration = Duration::from_millis(50);

enum Notice {
    Attention {
        event: AttentionEvent,
        repo: PathBuf,
        muted: bool,
    },
    Dismiss(SessionId),
    ClientView {
        view: ClientView,
        outbox: Arc<Outbox>,
    },
    ClientGone(ClientId),
}

pub(crate) struct Notifier {
    notices: mpsc::SyncSender<Notice>,
    worker: Option<JoinHandle<()>>,
    stopped: AtomicBool,
}

impl Notifier {
    pub(crate) fn spawn(
        target: NotificationTarget,
        loader: ConfigLoader,
        on_click: impl Fn(SessionId) + Send + 'static,
    ) -> Self {
        let (notices, pending) = mpsc::sync_channel(QUEUE_CAPACITY);
        let worker = std::thread::Builder::new()
            .name("orch-notifier".into())
            .spawn(move || {
                let sink: Box<dyn NotificationSink> = match target {
                    NotificationTarget::Desktop => Box::new(FreedesktopSink::spawn(on_click)),
                    NotificationTarget::Log(path) => Box::new(LogSink::open(path, on_click)),
                };
                Delivery::new(sink, loader).run(pending);
            })
            .inspect_err(|err| eprintln!("orch daemon: notifications disabled: {err}"))
            .ok();
        Self {
            notices,
            worker,
            stopped: AtomicBool::new(false),
        }
    }

    pub(crate) fn attention(&self, event: AttentionEvent, repo: PathBuf, muted: bool) {
        self.send(Notice::Attention { event, repo, muted });
    }

    pub(crate) fn dismiss(&self, session: SessionId) {
        self.send(Notice::Dismiss(session));
    }

    pub(crate) fn client_view(&self, view: ClientView, outbox: Arc<Outbox>) {
        self.send(Notice::ClientView { view, outbox });
    }

    pub(crate) fn client_gone(&self, client: ClientId) {
        self.send(Notice::ClientGone(client));
    }

    fn send(&self, notice: Notice) {
        match self.notices.try_send(notice) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                eprintln!("orch daemon: notification dropped: the notifier is behind");
            }
            Err(TrySendError::Disconnected(_)) => {
                if !self.stopped.swap(true, Ordering::Relaxed) {
                    let why = match self.worker.as_ref().is_none_or(JoinHandle::is_finished) {
                        true => "has exited",
                        false => "stopped receiving",
                    };
                    eprintln!("orch daemon: notifications stopped: the notifier thread {why}");
                }
            }
        }
    }
}

struct Delivery {
    sink: Box<dyn NotificationSink>,
    policy: NotificationPolicy,
    loader: ConfigLoader,
    configs: HashMap<PathBuf, (Instant, Notifications)>,
    client_views: BTreeMap<ClientId, (ClientView, Arc<Outbox>)>,
}

impl Delivery {
    fn new(sink: Box<dyn NotificationSink>, loader: ConfigLoader) -> Self {
        Self {
            sink,
            policy: NotificationPolicy::new(),
            loader,
            configs: HashMap::new(),
            client_views: BTreeMap::new(),
        }
    }

    fn run(mut self, pending: mpsc::Receiver<Notice>) {
        for notice in pending {
            let desktop = match notice {
                Notice::Attention { event, repo, muted } => self.attention(event, &repo, muted),
                Notice::Dismiss(session) => self.policy.dismiss(&session),
                Notice::ClientView { view, outbox } => {
                    self.client_views.insert(view.id, (view, outbox));
                    Vec::new()
                }
                Notice::ClientGone(client) => {
                    self.client_views.remove(&client);
                    Vec::new()
                }
            };
            for action in &desktop {
                self.sink.apply(action);
            }
        }
    }

    fn attention(&mut self, event: AttentionEvent, repo: &Path, muted: bool) -> Vec<DesktopAction> {
        let notifications = self.notifications(repo);
        let views: Vec<ClientView> = self
            .client_views
            .values()
            .map(|(view, _)| view.clone())
            .collect();
        let session = event.session.clone();
        let context = AttentionContext {
            muted,
            notifications: &notifications,
            clients: &views,
        };
        let actions = self.policy.on_attention(event, context);
        for ring in actions.bell {
            if let Some((_, outbox)) = self.client_views.get(&ring.client) {
                outbox.send(FromDaemon::Ring {
                    session: session.clone(),
                    title: ring.title,
                    body: ring.body,
                });
            }
        }
        actions.desktop
    }

    fn notifications(&mut self, repo: &Path) -> Notifications {
        if let Some((read_at, notifications)) = self.configs.get(repo)
            && read_at.elapsed() < CONFIG_TTL
        {
            return notifications.clone();
        }
        let notifications = match self.loader.repo(repo, None) {
            Ok(config) => config.notifications().clone(),
            Err(_) => self
                .loader
                .global()
                .map(|global| global.notifications)
                .unwrap_or_default(),
        };
        self.configs
            .insert(repo.to_path_buf(), (Instant::now(), notifications.clone()));
        notifications
    }
}

type Recorded = Mutex<RecordingSink>;

struct LogSink {
    log: Option<File>,
    recorded: Arc<Recorded>,
}

impl LogSink {
    fn open(path: PathBuf, on_click: impl Fn(SessionId) + Send + 'static) -> Self {
        let log = File::options()
            .create(true)
            .append(true)
            .open(&path)
            .inspect_err(|err| eprintln!("orch daemon: notification log: {err}"))
            .ok();
        let recorded = Arc::new(Recorded::default());
        let mut clicks = path.into_os_string();
        clicks.push(".clicks");
        let clicks = PathBuf::from(clicks);
        let watching = Arc::downgrade(&recorded);
        let _ = std::thread::Builder::new()
            .name("orch-notify-clicks".into())
            .spawn(move || watch_clicks(&clicks, &watching, on_click));
        Self { log, recorded }
    }
}

fn recorded(recorded: &Recorded) -> MutexGuard<'_, RecordingSink> {
    recorded
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn key_name(key: &NotificationKey) -> &str {
    match key {
        NotificationKey::Session(session) => session.as_str(),
        NotificationKey::Merged => "merged",
    }
}

fn parse_key(name: &str) -> Option<NotificationKey> {
    match name {
        "merged" => Some(NotificationKey::Merged),
        session => SessionId::parse(session).ok().map(NotificationKey::Session),
    }
}

impl NotificationSink for LogSink {
    fn apply(&mut self, action: &DesktopAction) {
        recorded(&self.recorded).apply(action);
        let entry = match action {
            DesktopAction::Show { key, notification } => json!({
                "action": "show",
                "key": key_name(key),
                "title": notification.title,
                "body": notification.body,
                "focus": notification.focus.as_str(),
            }),
            DesktopAction::Close { key } => json!({ "action": "close", "key": key_name(key) }),
        };
        if let Some(log) = &mut self.log {
            let _ = log.write_all(format!("{entry}\n").as_bytes());
        }
    }
}

fn watch_clicks(path: &Path, sink: &Weak<Recorded>, on_click: impl Fn(SessionId)) {
    let mut offset = 0;
    loop {
        std::thread::sleep(CLICK_POLL);
        let Some(sink) = sink.upgrade() else {
            return;
        };
        let Ok(mut file) = File::open(path) else {
            continue;
        };
        if file.metadata().is_ok_and(|meta| meta.len() < offset) {
            offset = 0;
        }
        if file.seek(SeekFrom::Start(offset)).is_err() {
            continue;
        }
        let mut reader = BufReader::new(&mut file);
        let mut line = String::new();
        while matches!(reader.read_line(&mut line), Ok(read) if read > 0 && line.ends_with('\n')) {
            offset += line.len() as u64;
            let focus = parse_key(line.trim()).and_then(|key| recorded(&sink).click(&key));
            if let Some(session) = focus {
                on_click(session);
            }
            line.clear();
        }
    }
}
