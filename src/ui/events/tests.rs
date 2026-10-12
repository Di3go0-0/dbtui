//! Key routing tests: names typed into panels, and the navigator.

mod typing_tests {
    use super::super::*;
    use crate::ui::state::ScriptsMode;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn press(state: &mut AppState, c: char) -> Action {
        handle_key(state, KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
    }

    #[test]
    fn new_collection_name_keeps_its_letters() {
        let mut state = AppState::new();
        state.focus = Focus::Sidebar;
        state.dialogs.group_creating = true;
        for c in "data ?".chars() {
            press(&mut state, c);
        }
        assert_eq!(state.dialogs.group_rename_buf, "data ?");
        assert!(
            state.overlay.is_none(),
            "`a` must not open the connection dialog"
        );
        assert!(state.dialogs.group_creating);
    }

    #[test]
    fn renaming_inside_the_oil_navigator_keeps_its_letters() {
        use crate::ui::state::OilState;
        let mut state = AppState::new();
        state.focus = Focus::Sidebar;
        state.oil = Some(OilState::new(Focus::Sidebar));
        state.dialogs.conn_renaming = Some("Singularidad".to_string());
        state.dialogs.conn_rename_buf = "Singul".to_string();
        for c in "aridad F".chars() {
            press(&mut state, c);
        }
        assert_eq!(state.dialogs.conn_rename_buf, "Singularidad F");
        assert!(state.overlay.is_none(), "`a` and `F` belong to the name");
        assert!(state.oil.is_some(), "oil stays open while renaming");
        assert!(
            !state.leader.pending,
            "Space is part of the name, not the leader"
        );
    }

    #[test]
    fn new_script_name_inside_the_navigator_keeps_its_spaces() {
        use crate::ui::state::{OilPane, OilState};
        let mut state = AppState::new();
        state.focus = Focus::TabContent;
        let mut oil = OilState::new(Focus::TabContent);
        oil.pane = OilPane::Scripts;
        state.oil = Some(oil);
        state.scripts.mode = ScriptsMode::Insert { buf: String::new() };
        for c in "my script".chars() {
            press(&mut state, c);
        }
        assert!(
            matches!(&state.scripts.mode, ScriptsMode::Insert { buf } if buf == "my script"),
            "the space is part of the name"
        );
        assert!(!state.leader.pending, "Space must not start the leader");
    }

    #[test]
    fn new_script_name_keeps_its_letters() {
        let mut state = AppState::new();
        state.focus = Focus::ScriptsPanel;
        state.scripts.mode = ScriptsMode::Insert { buf: String::new() };
        for c in "a-F".chars() {
            press(&mut state, c);
        }
        assert!(
            matches!(&state.scripts.mode, ScriptsMode::Insert { buf } if buf == "a-F"),
            "{:?}",
            state.overlay.is_some()
        );
        assert!(state.oil.is_none(), "`-` must not open the oil navigator");
        assert!(state.overlay.is_none());
    }
}

mod oil_diagram_tests {
    use super::super::*;
    use crate::ui::state::{LeafKind, OilState};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    #[test]
    fn d_in_the_navigator_opens_the_diagram_and_closes_it() {
        let mut state = AppState::new();
        state.focus = Focus::Sidebar;
        state.sidebar.tree.push(TreeNode::Connection {
            name: "pg".into(),
            expanded: true,
            status: crate::ui::state::ConnStatus::Connected,
        });
        state.sidebar.tree.push(TreeNode::Leaf {
            name: "orders".into(),
            schema: "tenants.public".into(),
            catalog: Some("tenants".into()),
            kind: LeafKind::Table,
            valid: true,
            privilege: crate::core::models::ObjectPrivilege::Full,
        });
        state.sidebar.tree_state.cursor = 1;
        state.oil = Some(OilState::new(Focus::Sidebar));
        let action = handle_key(
            &mut state,
            KeyEvent::new(KeyCode::Char('D'), KeyModifiers::NONE),
        );
        assert!(
            matches!(&action, Action::OpenDiagram { conn_name, schema, focus }
                if conn_name == "pg" && schema == "tenants.public" && focus.as_deref() == Some("orders")),
            "D on a table opens its schema's diagram centred on it"
        );
        assert!(state.oil.is_none(), "the navigator closes to show the tab");
    }

    #[test]
    fn leader_v_d_in_the_navigator_opens_the_diagram_and_closes_it() {
        let mut state = AppState::new();
        state.focus = Focus::Sidebar;
        state.sidebar.tree.push(TreeNode::Connection {
            name: "pg".into(),
            expanded: true,
            status: crate::ui::state::ConnStatus::Connected,
        });
        state.sidebar.tree.push(TreeNode::Schema {
            name: "tenants.public".into(),
            catalog: Some("tenants".into()),
            expanded: false,
        });
        state.sidebar.tree_state.cursor = 1;
        state.oil = Some(OilState::new(Focus::Sidebar));
        let mut last = Action::None;
        for c in [vimltui::LEADER_KEY, 'v', 'd'] {
            last = handle_key(
                &mut state,
                KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE),
            );
        }
        assert!(
            matches!(&last, Action::OpenDiagram { schema, focus, .. }
                if schema == "tenants.public" && focus.is_none()),
            "the schema's whole diagram"
        );
        assert!(
            state.oil.is_none(),
            "the navigator closes to show the diagram"
        );
        assert!(!state.leader.v_pending && !state.leader.pending);
    }
}
