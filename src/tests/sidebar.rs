use std::collections::HashSet;

use crate::{
    app::sidebar::{collapsible_folder_paths_under, folder_tree_rows},
    model::IndexedFolder,
};

fn folder(path: &str) -> IndexedFolder {
    IndexedFolder {
        path: path.into(),
        enabled: true,
        message_count: 1,
    }
}

#[test]
fn folder_paths_are_rendered_as_a_tree() {
    let folders = [
        folder("Mailbox/Projects/Alpha"),
        folder("Mailbox/Inbox"),
        folder("Archive"),
    ];
    let rows = folder_tree_rows(&folders, &HashSet::new());
    let labels: Vec<_> = rows.iter().map(|row| (row.depth, row.name)).collect();

    assert_eq!(
        labels,
        [
            (0, "Archive"),
            (0, "Mailbox"),
            (1, "Inbox"),
            (1, "Projects"),
            (2, "Alpha"),
        ]
    );
    assert_eq!(rows[1].folder_paths.len(), 2);
    assert_eq!(rows[2].folder_paths, ["Mailbox/Inbox"]);
}

#[test]
fn collapsed_folder_hides_all_descendants() {
    let folders = [folder("Mailbox/Projects/Alpha"), folder("Mailbox/Inbox")];
    let rows = folder_tree_rows(&folders, &HashSet::from(["Mailbox".into()]));

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].name, "Mailbox");
    assert!(rows[0].collapsed);
    assert_eq!(rows[0].folder_paths.len(), 2);
}

#[test]
fn collapsible_folder_paths_under_finds_all_branch_nodes_in_subtree() {
    let folders = [
        folder("Mailbox/Projects/Alpha/Specs"),
        folder("Mailbox/Projects/Beta"),
        folder("Mailbox/Inbox"),
        folder("Archive/2025/Q1"),
    ];

    assert_eq!(
        collapsible_folder_paths_under(&folders, "Mailbox"),
        vec![
            "Mailbox".to_string(),
            "Mailbox/Projects".to_string(),
            "Mailbox/Projects/Alpha".to_string(),
        ]
    );

    assert_eq!(
        collapsible_folder_paths_under(&folders, "Mailbox/Projects"),
        vec![
            "Mailbox/Projects".to_string(),
            "Mailbox/Projects/Alpha".to_string(),
        ]
    );

    // Leaf folder has no collapsible subdirectories
    assert!(collapsible_folder_paths_under(&folders, "Mailbox/Inbox").is_empty());

    // Empty prefix returns all collapsible directories in the PST
    assert_eq!(
        collapsible_folder_paths_under(&folders, ""),
        vec![
            "Archive".to_string(),
            "Archive/2025".to_string(),
            "Mailbox".to_string(),
            "Mailbox/Projects".to_string(),
            "Mailbox/Projects/Alpha".to_string(),
        ]
    );
}
