use std::path::Path;

fn read_ui(path: &str) -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(path)).unwrap()
}

#[test]
fn shell_topbar_tab_order_matches_electron() {
    let view_models = read_ui("src/view_models.rs");
    let controller = read_ui("src/controller.rs");

    let sessions_index = view_models.find("Self::Sessions => 0").unwrap();
    let db_index = view_models.find("Self::Db => 1").unwrap();
    let logs_index = view_models.find("Self::Logs => 2").unwrap();
    assert!(sessions_index < db_index && db_index < logs_index);

    let sessions_tab = controller.find("label: SharedString::from(\"Sessions\")").unwrap();
    let db_tab = controller.find("label: SharedString::from(\"DB\")").unwrap();
    let logs_tab = controller.find("label: SharedString::from(\"Logs\")").unwrap();
    assert!(sessions_tab < db_tab && db_tab < logs_tab);
}

#[test]
fn sessions_view_exposes_review_lane_filter_set() {
    let source = read_ui("src/view_models.rs");
    assert!(source.contains("Self::All => \"All\""));
    assert!(source.contains("Self::NeedsReview => \"Needs Review\""));
    assert!(source.contains("Self::TrainReady => \"Train Ready\""));
    assert!(source.contains("Self::HoldoutReady => \"Holdout Ready\""));
    assert!(source.contains("Self::Favorite => \"Favorites\""));
}

#[test]
fn export_surfaces_only_offer_train_and_holdout_targets() {
    let source = read_ui("ui/sessions_pane.slint");
    assert!(source.contains("train"));
    assert!(source.contains("holdout"));
    assert!(!source.contains("Export favorite"));
}

#[test]
fn topbar_search_is_sessions_only() {
    let source = read_ui("ui/shell.slint");
    assert!(source.contains("if root.active_route == 0"));
    assert!(source.contains("Search sessions…"));
}
