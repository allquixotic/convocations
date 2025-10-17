use rconv_core::config::{
    DurationOverride, RuntimeOverrides, FRIDAY_6_PRESET_ID, TUESDAY_7_PRESET_ID,
    TUESDAY_8_PRESET_ID,
};

// Integration tests for CLI runtime overrides and configuration logic.
// These tests verify that the runtime override system works correctly
// and that defaults are applied as expected.

#[test]
fn test_runtime_overrides_empty() {
    let overrides = RuntimeOverrides::default();
    assert!(
        overrides.is_empty(),
        "Default RuntimeOverrides should be empty"
    );
}

#[test]
fn test_runtime_overrides_last() {
    let mut overrides = RuntimeOverrides::default();
    overrides.last = Some(2);
    assert!(
        !overrides.is_empty(),
        "RuntimeOverrides with last set should not be empty"
    );
}

#[test]
fn test_runtime_overrides_preset() {
    let mut overrides = RuntimeOverrides::default();
    overrides.active_preset = Some(TUESDAY_7_PRESET_ID.to_string());
    assert!(!overrides.is_empty());
}

#[test]
fn test_runtime_overrides_duration() {
    let mut overrides = RuntimeOverrides::default();
    overrides.duration_override = Some(DurationOverride {
        enabled: true,
        hours: 2.5,
    });
    assert!(!overrides.is_empty());
}

#[test]
fn test_runtime_overrides_ai_corrections() {
    let mut overrides = RuntimeOverrides::default();
    overrides.use_ai_corrections = Some(false);
    assert!(!overrides.is_empty());
}

#[test]
fn test_runtime_overrides_multiple_fields() {
    let mut overrides = RuntimeOverrides::default();
    overrides.last = Some(1);
    overrides.dry_run = Some(true);
    overrides.use_ai_corrections = Some(false);
    overrides.keep_original_output = Some(true);
    overrides.show_diff = Some(false);

    assert!(!overrides.is_empty());
}

#[test]
fn test_preset_ids_consistency() {
    // Verify preset ID constants are consistent
    assert_eq!(TUESDAY_7_PRESET_ID, "tuesday-7pm");
    assert_eq!(TUESDAY_8_PRESET_ID, "tuesday-8pm");
    assert_eq!(FRIDAY_6_PRESET_ID, "friday-6pm");
}

#[test]
fn test_duration_override_validation() {
    // Test valid duration
    let valid = DurationOverride {
        enabled: true,
        hours: 1.5,
    };
    assert!(valid.hours.is_finite());
    assert!(valid.hours > 0.0);

    // Test edge cases
    let one_hour = DurationOverride {
        enabled: true,
        hours: 1.0,
    };
    assert!(one_hour.hours >= 1.0);

    let two_hours = DurationOverride {
        enabled: true,
        hours: 2.0,
    };
    assert!(two_hours.hours >= 1.0);
}

#[test]
fn test_runtime_overrides_ai_corrections_alias() {
    // Test that both use_llm and use_ai_corrections work
    let mut overrides = RuntimeOverrides::default();

    // Set use_llm
    overrides.use_llm = Some(false);
    assert!(!overrides.is_empty());

    // Set use_ai_corrections
    overrides.use_ai_corrections = Some(true);
    assert!(!overrides.is_empty());

    // Verify both can coexist
    assert_eq!(overrides.use_llm, Some(false));
    assert_eq!(overrides.use_ai_corrections, Some(true));
}

#[test]
fn test_runtime_overrides_diff_settings() {
    let mut overrides = RuntimeOverrides::default();

    // Test no_diff
    overrides.no_diff = Some(true);
    assert!(!overrides.is_empty());

    // Test show_diff (inverse of no_diff)
    overrides.show_diff = Some(false);
    assert_eq!(overrides.no_diff, Some(true));
    assert_eq!(overrides.show_diff, Some(false));
}

#[test]
fn test_runtime_overrides_output_settings() {
    let mut overrides = RuntimeOverrides::default();

    // Test keep_orig
    overrides.keep_orig = Some(true);
    assert!(!overrides.is_empty());

    // Test keep_original_output (should be the same)
    overrides.keep_original_output = Some(true);
    assert_eq!(overrides.keep_orig, Some(true));
    assert_eq!(overrides.keep_original_output, Some(true));
}

#[test]
fn test_optional_field_overrides() {
    let mut overrides = RuntimeOverrides::default();

    // Test optional string fields
    overrides.infile = Some("/path/to/ChatLog.log".to_string());
    overrides.outfile = Some(Some("output.txt".to_string()));
    overrides.start = Some(Some("2024-10-15T22:00:00".to_string()));
    overrides.end = Some(Some("2024-10-16T00:25:00".to_string()));

    assert!(!overrides.is_empty());
    assert_eq!(overrides.infile, Some("/path/to/ChatLog.log".to_string()));
    assert_eq!(overrides.outfile, Some(Some("output.txt".to_string())));
}

#[test]
fn test_processing_toggles() {
    let mut overrides = RuntimeOverrides::default();

    // Test format_dialogue toggle
    overrides.format_dialogue = Some(false);
    assert!(!overrides.is_empty());

    // Test cleanup toggle
    overrides.cleanup = Some(false);
    assert!(!overrides.is_empty());

    assert_eq!(overrides.format_dialogue, Some(false));
    assert_eq!(overrides.cleanup, Some(false));
}

#[test]
fn test_preset_shortcut_flags() {
    // These would normally be tested via CLI parsing, but we can verify the constants
    use rconv_core::config::SATURDAY_PRESET_ID;

    // Verify all preset IDs are unique
    let preset_ids = vec![
        SATURDAY_PRESET_ID,
        TUESDAY_7_PRESET_ID,
        TUESDAY_8_PRESET_ID,
        FRIDAY_6_PRESET_ID,
    ];

    let unique_count = preset_ids
        .iter()
        .collect::<std::collections::HashSet<_>>()
        .len();
    assert_eq!(
        unique_count,
        preset_ids.len(),
        "All preset IDs should be unique"
    );
}
