#[test]
fn selection_does_not_invoke_layout_mutating_rmux_commands() {
    let sources = [
        include_str!("../src/main.rs"),
        include_str!("../src/lib.rs"),
    ];
    let forbidden_commands = [
        "swap-pane",
        "resize-pane",
        "resize-window",
        "split-window",
        "join-pane",
        "break-pane",
        "select-layout",
        "new-window",
        "kill-pane",
    ];

    for source in sources {
        for command in forbidden_commands {
            assert!(
                !source.contains(&format!("\"{command}\"")),
                "selection must not invoke the layout-mutating command {command}"
            );
        }
    }
}
