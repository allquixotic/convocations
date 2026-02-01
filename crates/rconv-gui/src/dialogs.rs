//! File and folder dialog utilities

use std::path::PathBuf;

/// Open a file picker dialog
pub fn pick_file(title: &str, filter_name: &str, extensions: &[&str]) -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title(title)
        .add_filter(filter_name, extensions)
        .pick_file()
}

/// Open a folder picker dialog
pub fn pick_folder(title: &str) -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title(title)
        .pick_folder()
}

/// Pick chatlog file
pub fn pick_chatlog() -> Option<PathBuf> {
    pick_file("Select ChatLog File", "Log Files", &["txt", "log", "chatlog"])
}

/// Pick processed input file
pub fn pick_processed_input() -> Option<PathBuf> {
    pick_file("Select Processed Input File", "Text Files", &["txt"])
}

/// Pick output directory
pub fn pick_output_directory() -> Option<PathBuf> {
    pick_folder("Select Output Directory")
}

/// Save output file
pub fn save_output_file() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title("Save Output File")
        .add_filter("Text Files", &["txt"])
        .save_file()
}
