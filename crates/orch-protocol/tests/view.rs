use orch_protocol::PhaseView;

#[test]
fn only_active_and_pr_open_sessions_are_live() {
    let live: Vec<PhaseView> = [
        PhaseView::SettingUp,
        PhaseView::SetupFailed,
        PhaseView::Active,
        PhaseView::PrOpen,
        PhaseView::Suspended,
        PhaseView::Landed,
        PhaseView::Discarded,
    ]
    .into_iter()
    .filter(|phase| phase.is_live())
    .collect();
    assert_eq!(live, [PhaseView::Active, PhaseView::PrOpen]);
}
