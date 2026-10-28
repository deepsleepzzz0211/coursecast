//! Seam: `cc_core::timeline::{plan, fingerprint}` — the one and only timeline formula
//! (spec: 第 i 页画面时长 = d_i + g（i<N）；第 N 页 = d_N；T = Σd_i + (N-1)·g；g = 0.6).
//! Expected values are the timeline spike's worked example (independent source of truth).

use cc_core::timeline::{self, GAP_SEC};

const SPIKE_DURATIONS: [f64; 5] = [3.0, 2.0, 4.0, 2.5, 3.5];

#[test]
fn gap_constant_is_06s() {
    assert!((GAP_SEC - 0.6).abs() < 1e-12);
}

#[test]
fn plan_matches_timeline_spike_expectations() {
    let p = timeline::plan(&SPIKE_DURATIONS);

    let display: Vec<f64> = p.pages.iter().map(|pg| pg.display_sec).collect();
    let expected_display = [3.6, 2.6, 4.6, 3.1, 3.5];
    for (got, want) in display.iter().zip(expected_display) {
        assert!((got - want).abs() < 1e-9, "display {got} != {want}");
    }

    // turn to page i+1 happens after page i's audio + gap: 3.6, 6.2, 10.8, 13.9
    let expected_turns = [3.6, 6.2, 10.8, 13.9];
    assert_eq!(p.turn_points_sec.len(), 4);
    for (got, want) in p.turn_points_sec.iter().zip(expected_turns) {
        assert!((got - want).abs() < 1e-9, "turn {got} != {want}");
    }

    // T = 15.0 + 4*0.6 = 17.400
    assert!((p.total_sec - 17.4).abs() < 1e-9, "total {}", p.total_sec);

    for (i, pg) in p.pages.iter().enumerate() {
        assert_eq!(pg.page, (i + 1) as u32);
        assert!((pg.audio_sec - SPIKE_DURATIONS[i]).abs() < 1e-12);
    }
}

#[test]
fn single_page_has_no_turn_points_and_no_gap() {
    let p = timeline::plan(&[5.0]);
    assert_eq!(p.pages.len(), 1);
    assert_eq!(p.turn_points_sec.len(), 0);
    assert!((p.total_sec - 5.0).abs() < 1e-9);
    assert!((p.pages[0].display_sec - 5.0).abs() < 1e-9);
}

#[test]
fn fingerprint_is_stable_and_hex_sha256() {
    let a = timeline::fingerprint(&SPIKE_DURATIONS);
    let b = timeline::fingerprint(&SPIKE_DURATIONS);
    assert_eq!(a, b, "same inputs must give the same fingerprint");
    let hex = a
        .strip_prefix("sha256:")
        .expect("fingerprint prefixed sha256:");
    assert_eq!(hex.len(), 64);
    assert!(hex.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn fingerprint_binds_durations_and_page_count() {
    let base = timeline::fingerprint(&SPIKE_DURATIONS);

    // one page's audio got shorter -> fingerprint must change
    let mut changed = SPIKE_DURATIONS;
    changed[2] = 4.1;
    assert_ne!(base, timeline::fingerprint(&changed));

    // a page went missing -> page count changed -> fingerprint must change
    let fewer = &SPIKE_DURATIONS[..4];
    assert_ne!(base, timeline::fingerprint(fewer));
}
