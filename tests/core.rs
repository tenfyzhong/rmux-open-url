use pretty_assertions::assert_eq;
use rmux_open_url::{
    ActionInput, Extractor, build_copy_action, build_open_action, filter_candidates, fuzzy_score,
    parse_show_options, strip_ansi,
};

#[test]
fn extracts_standard_urls() {
    let extractor = Extractor::with_defaults().unwrap();
    let urls = extractor.extract(
        "see https://example.com/path?q=1 and ftp://files.example.com/a.tar.gz and file:///tmp/x",
    );
    assert_eq!(
        urls,
        vec![
            "https://example.com/path?q=1",
            "ftp://files.example.com/a.tar.gz",
            "file:///tmp/x",
        ]
    );
}

#[test]
fn converts_git_ssh_urls() {
    let extractor = Extractor::with_defaults().unwrap();
    assert_eq!(
        extractor.extract("clone git@github.com:user/repo.git and ssh://git@github.com/org/repo"),
        vec![
            "https://github.com/user/repo",
            "https://github.com/org/repo"
        ]
    );
}

#[test]
fn prefixes_www_and_ip_addresses() {
    let extractor = Extractor::with_defaults().unwrap();
    assert_eq!(
        extractor.extract("open www.example.com or 192.168.1.1:8080/api or 10.0.0.1"),
        vec![
            "http://www.example.com",
            "http://192.168.1.1:8080/api",
            "http://10.0.0.1",
        ]
    );
}

#[test]
fn converts_github_shorthand() {
    let extractor = Extractor::with_defaults().unwrap();
    assert_eq!(
        extractor.extract("check 'user/repo' and \"my-org/my-repo\""),
        vec![
            "https://github.com/user/repo",
            "https://github.com/my-org/my-repo",
        ]
    );
}

#[test]
fn converts_bare_github_shorthand() {
    let extractor = Extractor::with_defaults().unwrap();
    assert_eq!(
        extractor.extract("see abcd/efgh or my-org/my-repo here"),
        vec![
            "https://github.com/abcd/efgh",
            "https://github.com/my-org/my-repo",
        ]
    );
}

#[test]
fn bare_github_shorthand_requires_whitespace_delimiters() {
    let extractor = Extractor::with_defaults().unwrap();
    // Segments of a path or URL are not standalone words.
    assert_eq!(
        extractor.extract("~/.config/rmux/extensions and https://github.com/user/repo"),
        vec!["https://github.com/user/repo"]
    );
    // A trailing period means the word does not end in whitespace.
    assert_eq!(
        extractor.extract("open abcd/efgh. now"),
        Vec::<String>::new()
    );
}

#[test]
fn adjacent_bare_shorthand_matches() {
    let extractor = Extractor::with_defaults().unwrap();
    assert_eq!(
        extractor.extract("a/b c/d"),
        vec!["https://github.com/a/b", "https://github.com/c/d"]
    );
}

#[test]
fn bare_github_shorthand_matches_at_line_boundaries() {
    let extractor = Extractor::with_defaults().unwrap();
    assert_eq!(
        extractor.extract("abcd/efgh\n\nsee ghij/klm at line end"),
        vec![
            "https://github.com/abcd/efgh",
            "https://github.com/ghij/klm",
        ]
    );
}

#[test]
fn strips_trailing_git_from_github_urls() {
    let extractor = Extractor::with_defaults().unwrap();
    // Clone-style URLs and shorthand alike lose the trailing .git.
    assert_eq!(
        extractor.extract("clone https://github.com/a/b.git and see c/d.git"),
        vec!["https://github.com/a/b", "https://github.com/c/d"]
    );
    // Other hosts keep the .git suffix.
    assert_eq!(
        extractor.extract("clone https://gitlab.com/org/repo.git"),
        vec!["https://gitlab.com/org/repo.git"]
    );
}

#[test]
fn bare_repo_inside_full_url_is_not_duplicated() {
    let extractor = Extractor::with_defaults().unwrap();
    assert_eq!(
        extractor.extract(
            "https://github.com/tenfyzhong/rmux-fastcopy and git@github.com:user/repo.git"
        ),
        vec![
            "https://github.com/tenfyzhong/rmux-fastcopy",
            "https://github.com/user/repo",
        ]
    );
}

#[test]
fn drops_www_contained_in_full_url() {
    let extractor = Extractor::with_defaults().unwrap();
    assert_eq!(
        extractor.extract("visit https://www.example.com/path"),
        vec!["https://www.example.com/path"]
    );
}

#[test]
fn deduplicates_repeated_urls() {
    let extractor = Extractor::with_defaults().unwrap();
    assert_eq!(
        extractor.extract("go https://a.com then https://a.com"),
        vec!["https://a.com"]
    );
}

#[test]
fn deduplicates_normalized_and_literal_urls() {
    let extractor = Extractor::with_defaults().unwrap();
    // The Git SSH pattern and the plain URL pattern both produce the same
    // https URL (and the .git suffix is dropped); it must be listed once.
    assert_eq!(
        extractor.extract("git@github.com:user/repo.git https://github.com/user/repo.git"),
        vec!["https://github.com/user/repo"]
    );
}

#[test]
fn custom_pattern_with_substitution() {
    let mut extractor = Extractor::with_defaults().unwrap();
    extractor
        .with_custom(r"[A-Z]+-\d+", Some("https://jira.example.com/browse/$0"))
        .unwrap();
    let urls = extractor.extract("fix TICKET-1234 and see https://example.com");
    assert_eq!(
        urls,
        vec![
            "https://jira.example.com/browse/TICKET-1234",
            "https://example.com",
        ]
    );
}

#[test]
fn custom_pattern_defaults_to_whole_match() {
    let mut extractor = Extractor::with_defaults().unwrap();
    extractor.with_custom(r"\b\d{4}\b", None).unwrap();
    assert_eq!(extractor.extract("year 2026"), vec!["2026"]);
}

#[test]
fn strips_ansi_escape_sequences() {
    assert_eq!(
        strip_ansi("\x1b[38;2;128;128;128mhttps://example.com\x1b[39m"),
        "https://example.com"
    );
    assert_eq!(
        strip_ansi("\x1b]0;tab title\x07https://example.com"),
        "https://example.com"
    );
}

#[test]
fn fuzzy_score_matches_subsequences() {
    assert!(fuzzy_score("gh", "https://github.com/user/repo").is_some());
    assert!(fuzzy_score("GITHUB.COM", "https://github.com/user").is_some());
    assert!(fuzzy_score("zzz", "https://github.com/user").is_none());
    assert_eq!(fuzzy_score("", "anything"), Some(0));
}

#[test]
fn filter_ranks_best_matches_first() {
    let urls = vec![
        "https://example.com".to_owned(),
        "https://github.com/a".to_owned(),
        "https://github.com/b".to_owned(),
    ];
    assert_eq!(filter_candidates("github.com", &urls), vec![1, 2]);
    assert_eq!(filter_candidates("", &urls), vec![0, 1, 2]);
    assert_eq!(filter_candidates("zzz", &urls), Vec::<usize>::new());
}

#[test]
fn parses_show_options_output() {
    let output = concat!(
        "@open-url-open firefox\n",
        "@open-url-history-limit \"2000\"\n",
        "@open-url-copy-cmd \"xclip -selection clipboard\"\n",
        "@open-url-custom-pat \\\\b[A-Z]+-\\\\d+\\\\b\n",
        "@open-url-bind u\n",
    );
    let options = parse_show_options(output);
    assert_eq!(options[0], ("@open-url-open".into(), "firefox".into()));
    assert_eq!(
        options[1],
        ("@open-url-history-limit".into(), "2000".into())
    );
    assert_eq!(
        options[2],
        (
            "@open-url-copy-cmd".into(),
            "xclip -selection clipboard".into()
        )
    );
    assert_eq!(
        options[3],
        ("@open-url-custom-pat".into(), r"\b[A-Z]+-\d+\b".into())
    );
    assert_eq!(options[4], ("@open-url-bind".into(), "u".into()));
}

#[test]
fn copy_action_uses_stdin_without_placeholder() {
    assert_eq!(
        build_copy_action("pbcopy", "https://example.com").unwrap(),
        ActionInput {
            program: "pbcopy".into(),
            args: vec![],
            stdin: Some("https://example.com".into()),
        }
    );
    assert_eq!(
        build_copy_action("rmux load-buffer -", "https://example.com").unwrap(),
        ActionInput {
            program: "rmux".into(),
            args: vec!["load-buffer".into(), "-".into()],
            stdin: Some("https://example.com".into()),
        }
    );
}

#[test]
fn copy_action_replaces_placeholder() {
    assert_eq!(
        build_copy_action("echo {}", "https://example.com").unwrap(),
        ActionInput {
            program: "echo".into(),
            args: vec!["https://example.com".into()],
            stdin: None,
        }
    );
}

#[test]
fn open_action_appends_url_as_argument() {
    assert_eq!(
        build_open_action("xdg-open", "https://example.com").unwrap(),
        ActionInput {
            program: "xdg-open".into(),
            args: vec!["https://example.com".into()],
            stdin: None,
        }
    );
    assert_eq!(
        build_open_action("open {}", "https://example.com").unwrap(),
        ActionInput {
            program: "open".into(),
            args: vec!["https://example.com".into()],
            stdin: None,
        }
    );
}
