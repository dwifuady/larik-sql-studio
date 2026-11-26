use larik_update::{InstallKind, UpdateService, can_self_update};
use std::path::PathBuf;

#[test]
#[ignore = "hits the real GitHub API"]
fn live_check_reports_up_to_date_against_012() {
    let service = UpdateService::new(PathBuf::from(std::env::temp_dir()).join("larik-live-test"));
    let result = service.check();
    println!("status={:?} error={:?}", result.status, result.error);
    assert!(
        matches!(
            result.status,
            larik_update::CheckStatus::UpToDate | larik_update::CheckStatus::Available
        ),
        "live check failed: {:?}",
        result.error
    );
}

#[test]
fn install_kind_is_detected() {
    let service = UpdateService::new(PathBuf::from(std::env::temp_dir()));
    let kind = service.install_kind();
    println!("kind={kind:?} supported={}", can_self_update());
    assert!(matches!(
        kind,
        InstallKind::Installer | InstallKind::Portable
    ));
}
