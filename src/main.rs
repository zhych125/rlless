//! rlless - High-Performance Terminal Log Viewer
//!
//! A fast, memory-efficient terminal log viewer designed to handle extremely large files.

use anyhow::Result;
use clap::{Arg, ArgAction, Command};
use rlless::search::SearchOptions;
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging for development
    env_logger::init();

    // Parse command-line arguments
    let matches = Command::new("rlless")
        .version(rlless::VERSION)
        .about("A high-performance terminal log viewer for large files")
        .long_about(
            "rlless is a terminal-based log viewer that can handle extremely large files \
             (40GB+) with SIMD-optimized search and memory-efficient streaming.",
        )
        .arg(
            Arg::new("file")
                .help("Path to the log file to view")
                .required(true)
                .index(1),
        )
        .arg(
            Arg::new("ignore-case")
                .short('i')
                .long("ignore-case")
                .help("Perform case-insensitive searches by default")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("literal")
                .long("literal")
                .help("Treat search patterns as literal strings")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("regex")
                .long("regex")
                .help("Treat search patterns as regular expressions (default)")
                .action(ArgAction::SetTrue)
                .conflicts_with("literal"),
        )
        .arg(
            Arg::new("word")
                .long("word")
                .short('w')
                .help("Match whole words only")
                .action(ArgAction::SetTrue),
        )
        .get_matches();

    // Get the file path argument
    let file_path = PathBuf::from(
        matches
            .get_one::<String>("file")
            .expect("file argument is required"),
    );

    // Validate file exists
    if !file_path.exists() {
        anyhow::bail!("File does not exist: {}", file_path.display());
    }

    if !file_path.is_file() {
        anyhow::bail!("Path is not a regular file: {}", file_path.display());
    }

    // Initialize the Application and start the interactive event loop
    use rlless::render::ui::TerminalUI;
    use rlless::Application;

    let mut search_options = SearchOptions::default();
    if matches.get_flag("ignore-case") {
        search_options.case_sensitive = false;
    }
    if matches.get_flag("literal") {
        search_options.regex_mode = false;
    }
    if matches.get_flag("regex") {
        search_options.regex_mode = true;
    }
    if matches.get_flag("word") {
        search_options.whole_word = true;
    }

    let ui_renderer = Box::new(TerminalUI::new()?);
    let mut app = Application::new(&file_path, ui_renderer, search_options).await?;

    app.run().await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_version_constant() {
        // Ensure version is accessible
        assert!(!rlless::VERSION.is_empty());
    }
}
