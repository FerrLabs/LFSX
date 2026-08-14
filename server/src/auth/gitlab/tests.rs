use super::*;

#[test]
fn a_project_path_is_encoded_as_one_segment() {
    assert_eq!(urlencoding("FerrLabs"), "FerrLabs");
    assert_eq!(urlencoding("Idler-Survivor"), "Idler-Survivor");
    assert_eq!(
        urlencoding("groupe avec espace"),
        "groupe%20avec%20espace",
        "GitLab addresses a project by its encoded path, so anything else must be escaped"
    );
}
