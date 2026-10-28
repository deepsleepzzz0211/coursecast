//! Seam: the pure script-entry helpers (PRD §4.2 Step2) — whole-paste splitting on
//! `---` / `第N页` markers and the "自动均分" grouping when segments ≠ slide count.

use cc_core::command::script::{auto_balance, split_script};

#[test]
fn dash_lines_split_the_paste() {
    let text = "第一页讲稿\n---\n第二页讲稿";
    assert_eq!(split_script(text), vec!["第一页讲稿", "第二页讲稿"]);
}

#[test]
fn page_marker_lines_split_and_are_removed() {
    let text = "开场白\n第2页\n这里讲第二页\n第 3 页\n第三页内容";
    assert_eq!(
        split_script(text),
        vec!["开场白", "这里讲第二页", "第三页内容"]
    );
}

#[test]
fn markdown_heading_markers_also_split() {
    let text = "## 第1页\n正文一\n## 第 2 页\n正文二";
    assert_eq!(split_script(text), vec!["正文一", "正文二"]);
}

#[test]
fn no_markers_yields_one_segment() {
    assert_eq!(
        split_script("整段文字没有分隔符"),
        vec!["整段文字没有分隔符"]
    );
    assert_eq!(split_script("  \n"), Vec::<String>::new());
}

#[test]
fn balance_passthrough_when_counts_match() {
    let segs = vec!["a".to_string(), "b".to_string(), "c".to_string()];
    assert_eq!(auto_balance(segs.clone(), 3).unwrap(), segs);
}

#[test]
fn more_segments_than_pages_group_consecutively() {
    // 5 segments -> 2 pages: proportion says page1 gets 0..2, page2 gets 2..5
    let segs: Vec<String> = ["s1", "s2", "s3", "s4", "s5"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let pages = auto_balance(segs, 2).unwrap();
    assert_eq!(pages.len(), 2);
    assert_eq!(pages[0], "s1\ns2");
    assert_eq!(pages[1], "s3\ns4\ns5");
}

#[test]
fn fewer_segments_than_pages_leave_empty_pages() {
    let segs = vec!["a".to_string(), "b".to_string()];
    let pages = auto_balance(segs, 4).unwrap();
    assert_eq!(
        pages,
        vec![
            "a".to_string(),
            "b".to_string(),
            String::new(),
            String::new()
        ]
    );
}

#[test]
fn zero_pages_is_rejected() {
    let err = auto_balance(vec!["a".to_string()], 0).unwrap_err();
    assert_eq!(err.code(), "invalid_argument");
}
