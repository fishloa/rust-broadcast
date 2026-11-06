use workspace_guards::workspace_members;

/// Fails if a workspace member is neither in `table` nor in `exempt`, or if a
/// named crate is not a workspace member (a rename must update the table).
pub fn assert_table_covers_members(family: &str, table: &[&str], exempt: &[&str]) {
    let members = workspace_members();
    let mut problems = Vec::new();
    for m in &members {
        if m != "workspace-guards" && !table.contains(&m.as_str()) && !exempt.contains(&m.as_str())
        {
            problems.push(format!(
                "{family}: workspace member `{m}` is in neither the guard table nor its exempt list"
            ));
        }
    }
    for name in table.iter().chain(exempt) {
        if !members.iter().any(|m| m == name) {
            problems.push(format!("{family}: `{name}` is not a workspace member"));
        }
    }
    for name in table {
        if exempt.contains(name) {
            problems.push(format!(
                "{family}: `{name}` is both in the table and exempt"
            ));
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// Fails if a crate named in a scoped guard table is not a workspace member
/// (a rename must update the table).
pub fn assert_names_are_members(family: &str, names: &[&str]) {
    let members = workspace_members();
    let missing: Vec<String> = names
        .iter()
        .filter(|n| !members.iter().any(|m| m == **n))
        .map(|n| format!("{family}: `{n}` is not a workspace member"))
        .collect();
    assert!(missing.is_empty(), "{}", missing.join("\n"));
}
