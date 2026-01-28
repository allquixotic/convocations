use std::fs;
use std::sync::{Arc, Mutex};

use rconv_core::{
    ConvocationsConfig, StageProgressEvent, StageProgressEventKind, run_with_config,
    run_with_config_with_progress,
};
use tempfile::tempdir;

const SAMPLE_LOG: &str = "\
2025-01-04T22:00:00.000-05:00 0,Character One,Hello there\n\
2025-01-04T22:01:15.125-05:00 6,Character Two,gestures gracefully\n";

fn base_config(infile: &str, outfile: &str) -> ConvocationsConfig {
    let mut config = ConvocationsConfig::default();
    config.infile = infile.to_string();
    config.outfile = Some(outfile.to_string());
    config.start = Some("2025-01-04T21:30".to_string());
    config.end = Some("2025-01-04T23:30".to_string());
    config.use_llm = false;
    config.no_diff = true;
    config.keep_orig = false;
    config.cleanup = true;
    config.format_dialogue = true;
    config.openrouter_api_key = None;
    config.openrouter_model = rconv_core::curator::AUTO_SENTINEL.to_string();
    config
}

#[tokio::test]
async fn pipeline_formats_chatlog_output() {
    let temp = tempdir().expect("tempdir");
    let infile_path = temp.path().join("ChatLog.log");
    let outfile_path = temp.path().join("output.txt");

    fs::write(&infile_path, SAMPLE_LOG).expect("write fixture");

    let config = base_config(
        infile_path.to_string_lossy().as_ref(),
        outfile_path.to_string_lossy().as_ref(),
    );

    run_with_config(config)
        .await
        .expect("pipeline completed successfully");

    let output = fs::read_to_string(&outfile_path).expect("read output");
    let expected = "Character One says, \"Hello there.\"\nCharacter Two gestures gracefully.\n";
    assert_eq!(output, expected);
}

#[tokio::test]
async fn pipeline_emits_diff_event_when_llm_enabled() {
    let temp = tempdir().expect("tempdir");
    let infile_path = temp.path().join("ChatLog.log");
    let outfile_path = temp.path().join("output_llm.txt");

    fs::write(&infile_path, SAMPLE_LOG).expect("write fixture");

    let mut config = base_config(
        infile_path.to_string_lossy().as_ref(),
        outfile_path.to_string_lossy().as_ref(),
    );
    config.use_llm = true;
    config.no_diff = false;
    config.keep_orig = true;

    let diff_events: Arc<Mutex<Vec<StageProgressEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let collector = diff_events.clone();
    let callback = Arc::new(move |event: StageProgressEvent| {
        if matches!(event.kind, StageProgressEventKind::Diff) {
            collector.lock().unwrap().push(event);
        }
    });

    run_with_config_with_progress(config, callback)
        .await
        .expect("pipeline completed");

    let events = diff_events.lock().unwrap();
    assert_eq!(events.len(), 1, "expected exactly one diff event");
    let diff_payload = events[0]
        .diff
        .clone()
        .expect("diff payload should be present");
    assert!(
        diff_payload.contains("---"),
        "diff payload should include headers"
    );

    let unedited_path = temp.path().join("output_llm_unedited.txt");
    assert!(
        unedited_path.exists(),
        "unedited snapshot should be retained when keep_orig=true"
    );
}
