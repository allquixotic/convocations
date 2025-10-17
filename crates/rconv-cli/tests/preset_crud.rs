use rconv_core::config::{
    FileConfig, PresetDefinition, RuntimePreferences, UiPreferences, FRIDAY_6_PRESET_ID,
    SATURDAY_PRESET_ID, TUESDAY_7_PRESET_ID, TUESDAY_8_PRESET_ID,
};

#[test]
fn test_default_presets_exist() {
    // Load default config
    let config = FileConfig::default();

    // Verify all built-in presets exist
    assert!(
        config
            .presets
            .iter()
            .any(|p| p.id == SATURDAY_PRESET_ID && p.builtin),
        "Saturday preset should exist and be built-in"
    );
    assert!(
        config
            .presets
            .iter()
            .any(|p| p.id == TUESDAY_7_PRESET_ID && p.builtin),
        "Tuesday 7pm preset should exist and be built-in"
    );
    assert!(
        config
            .presets
            .iter()
            .any(|p| p.id == TUESDAY_8_PRESET_ID && p.builtin),
        "Tuesday 8pm preset should exist and be built-in"
    );
    assert!(
        config
            .presets
            .iter()
            .any(|p| p.id == FRIDAY_6_PRESET_ID && p.builtin),
        "Friday 6pm preset should exist and be built-in"
    );

    // Verify preset has all required fields
    let saturday_preset = config
        .presets
        .iter()
        .find(|p| p.id == SATURDAY_PRESET_ID)
        .unwrap();
    assert!(!saturday_preset.name.is_empty(), "Preset should have a name");
    assert!(
        !saturday_preset.weekday.is_empty(),
        "Preset should have a weekday"
    );
    assert!(
        !saturday_preset.timezone.is_empty(),
        "Preset should have a timezone"
    );
    assert!(
        !saturday_preset.start_time.is_empty(),
        "Preset should have a start_time"
    );
    assert!(
        saturday_preset.duration_minutes > 0,
        "Preset should have a positive duration"
    );
    assert!(
        !saturday_preset.file_prefix.is_empty(),
        "Preset should have a file_prefix"
    );
}

#[test]
fn test_preset_create() {
    let mut config = FileConfig::default();
    let initial_count = config.presets.len();

    // Create a new custom preset
    let custom_preset = PresetDefinition {
        id: "custom-event".to_string(),
        name: "Custom Event".to_string(),
        weekday: "wednesday".to_string(),
        timezone: "America/New_York".to_string(),
        start_time: "18:30".to_string(),
        duration_minutes: 90,
        file_prefix: "custom".to_string(),
        default_weeks_ago: 0,
        builtin: false,
    };

    config.presets.push(custom_preset.clone());

    // Verify the preset was added
    assert_eq!(
        config.presets.len(),
        initial_count + 1,
        "Preset count should increase"
    );
    assert!(
        config.presets.iter().any(|p| p.id == "custom-event"),
        "Custom preset should exist"
    );

    // Verify all fields
    let added = config
        .presets
        .iter()
        .find(|p| p.id == "custom-event")
        .unwrap();
    assert_eq!(added.name, "Custom Event");
    assert_eq!(added.weekday, "wednesday");
    assert_eq!(added.timezone, "America/New_York");
    assert_eq!(added.start_time, "18:30");
    assert_eq!(added.duration_minutes, 90);
    assert_eq!(added.file_prefix, "custom");
    assert_eq!(added.default_weeks_ago, 0);
    assert!(!added.builtin);
}

#[test]
fn test_preset_update() {
    let mut config = FileConfig::default();

    // Find a preset to update (use a built-in one for testing)
    let preset = config
        .presets
        .iter_mut()
        .find(|p| p.id == SATURDAY_PRESET_ID)
        .unwrap();

    // Update some fields
    let original_name = preset.name.clone();
    preset.name = "Updated Saturday Event".to_string();
    preset.default_weeks_ago = 1;

    // Verify updates
    let updated = config
        .presets
        .iter()
        .find(|p| p.id == SATURDAY_PRESET_ID)
        .unwrap();
    assert_ne!(updated.name, original_name);
    assert_eq!(updated.name, "Updated Saturday Event");
    assert_eq!(updated.default_weeks_ago, 1);
}

#[test]
fn test_preset_delete() {
    let mut config = FileConfig::default();

    // Add a custom preset
    config.presets.push(PresetDefinition {
        id: "deletable-preset".to_string(),
        name: "Deletable".to_string(),
        weekday: "thursday".to_string(),
        timezone: "America/New_York".to_string(),
        start_time: "20:00".to_string(),
        duration_minutes: 60,
        file_prefix: "del".to_string(),
        default_weeks_ago: 0,
        builtin: false,
    });

    let initial_count = config.presets.len();
    assert!(config.presets.iter().any(|p| p.id == "deletable-preset"));

    // Delete the preset
    config.presets.retain(|p| p.id != "deletable-preset");

    // Verify deletion
    assert_eq!(
        config.presets.len(),
        initial_count - 1,
        "Preset count should decrease"
    );
    assert!(
        !config.presets.iter().any(|p| p.id == "deletable-preset"),
        "Deleted preset should not exist"
    );
}

#[test]
fn test_runtime_defaults() {
    let runtime = RuntimePreferences::default();

    // Verify default values
    assert_eq!(
        runtime.chat_log_path,
        "~/Documents/Elder Scrolls Online/live/Logs/ChatLog.log",
        "Default chat log path should be correct"
    );
    assert_eq!(
        runtime.active_preset, SATURDAY_PRESET_ID,
        "Default preset should be Saturday"
    );
    assert_eq!(runtime.weeks_ago, 0, "Default weeks_ago should be 0");
    assert!(!runtime.dry_run, "Default dry_run should be false");
    assert!(
        runtime.use_ai_corrections,
        "Default use_ai_corrections should be true"
    );
    assert!(
        !runtime.keep_original_output,
        "Default keep_original_output should be false"
    );
    assert!(
        runtime.show_diff,
        "Default show_diff should be true"
    );
    assert!(
        runtime.cleanup_enabled,
        "Default cleanup_enabled should be true"
    );
    assert!(
        runtime.format_dialogue_enabled,
        "Default format_dialogue_enabled should be true"
    );
    assert!(
        runtime.outfile_override.is_none(),
        "Default outfile_override should be None"
    );
    assert!(
        !runtime.duration_override.enabled,
        "Default duration override should be disabled"
    );
    assert_eq!(
        runtime.duration_override.hours, 1.0,
        "Default duration hours should be 1.0"
    );
}

#[test]
fn test_ui_defaults() {
    let ui = UiPreferences::default();

    // Verify UI defaults
    assert_eq!(
        ui.theme,
        rconv_core::config::ThemePreference::Dark,
        "Default theme should be Dark"
    );
    assert!(!ui.show_technical_log, "Default show_technical_log should be false");
    assert!(
        ui.follow_technical_log,
        "Default follow_technical_log should be true"
    );
}

#[test]
fn test_preset_field_validation() {
    let config = FileConfig::default();

    // Verify all built-in presets have valid data
    for preset in &config.presets {
        assert!(!preset.id.is_empty(), "Preset ID should not be empty");
        assert!(!preset.name.is_empty(), "Preset name should not be empty");
        assert!(
            !preset.weekday.is_empty(),
            "Preset weekday should not be empty"
        );
        assert!(
            !preset.timezone.is_empty(),
            "Preset timezone should not be empty"
        );
        assert!(
            !preset.start_time.is_empty(),
            "Preset start_time should not be empty"
        );
        assert!(
            preset.duration_minutes > 0,
            "Preset duration_minutes should be positive"
        );
        assert!(
            !preset.file_prefix.is_empty(),
            "Preset file_prefix should not be empty"
        );

        // Validate weekday format (should be lowercase)
        assert_eq!(
            preset.weekday,
            preset.weekday.to_ascii_lowercase(),
            "Preset weekday should be lowercase"
        );

        // Validate time format (should be HH:MM)
        assert!(
            preset.start_time.contains(':'),
            "Preset start_time should contain ':'"
        );
    }
}

#[test]
fn test_duration_override_values() {
    use rconv_core::config::DurationOverride;

    // Test disabled override
    let disabled = DurationOverride::default();
    assert!(!disabled.enabled);
    assert_eq!(disabled.hours, 1.0);

    // Test enabled override with different hours
    let one_hour = DurationOverride {
        enabled: true,
        hours: 1.0,
    };
    assert!(one_hour.enabled);
    assert_eq!(one_hour.hours, 1.0);

    let two_hours = DurationOverride {
        enabled: true,
        hours: 2.0,
    };
    assert!(two_hours.enabled);
    assert_eq!(two_hours.hours, 2.0);

    let custom_hours = DurationOverride {
        enabled: true,
        hours: 2.5,
    };
    assert!(custom_hours.enabled);
    assert_eq!(custom_hours.hours, 2.5);
}
