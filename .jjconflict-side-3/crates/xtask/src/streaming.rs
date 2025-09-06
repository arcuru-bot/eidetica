use crate::GlobalOptions;
use crate::output::{DRY_RUN, FAILURE, SUCCESS};
use anyhow::{Context, Result};
use crossterm::terminal;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

type FailedTasksList = Arc<Mutex<Vec<(String, Vec<(&'static str, String)>)>>>;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::LinesStream;

/// A streaming task that captures command output in real-time
pub struct StreamingTask {
    pub name: String,
    pub command: Command,
}

impl StreamingTask {
    pub fn new(name: impl Into<String>, mut command: Command) -> Self {
        // Configure command for streaming
        command.stdout(std::process::Stdio::piped());
        command.stderr(std::process::Stdio::piped());

        Self {
            name: name.into(),
            command,
        }
    }
}

/// Manages multiple streaming command panels
pub struct StreamingRunner {
    multi: MultiProgress,
    main_pb: ProgressBar,
    main_message: String,
    global_options: GlobalOptions,
}

impl StreamingRunner {
    pub fn new(main_message: impl Into<String>, global_options: &GlobalOptions) -> Self {
        let multi = MultiProgress::new();
        let main_pb = multi.add(ProgressBar::new_spinner());

        main_pb.set_style(
            ProgressStyle::with_template("{spinner:.blue} {msg}")
                .unwrap()
                .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
        );

        let main_message = main_message.into();
        main_pb.set_message(main_message.clone());
        main_pb.enable_steady_tick(Duration::from_millis(120));

        Self {
            multi,
            main_pb,
            main_message,
            global_options: global_options.clone(),
        }
    }

    /// Run streaming tasks with real-time output panels
    pub async fn run_streaming_tasks(&self, tasks: Vec<StreamingTask>) -> Result<()> {
        if self.global_options.dry_run {
            return self.run_dry_run(tasks).await;
        }

        // Print header (skip if single task with same name - avoids duplication)
        self.main_pb.finish_and_clear();
        let skip_header = tasks.len() == 1 && tasks[0].name == self.main_message;
        if !skip_header {
            println!("{}...", self.main_message);
        }

        let task_count = tasks.len();
        let mut handles = Vec::new();
        let mut task_panels = HashMap::new();
        let failed_tasks: FailedTasksList = Arc::new(Mutex::new(Vec::new()));

        // Create progress bars for all tasks
        for (i, task) in tasks.into_iter().enumerate() {
            let pb = self.multi.add(ProgressBar::new_spinner());
            pb.set_style(
                ProgressStyle::with_template("  {spinner:.dim} {msg:.50}")
                    .unwrap()
                    .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
            );
            pb.set_message(task.name.clone());
            pb.enable_steady_tick(Duration::from_millis(80));

            let panel_id = format!("task_{}", i);
            task_panels.insert(panel_id.clone(), pb.clone());

            // Spawn task
            let task_name = task.name.clone();
            let task_name_for_handle = task_name.clone();
            let failed_tasks = Arc::clone(&failed_tasks);
            let global_options = self.global_options.clone();
            let output_buffer: Arc<Mutex<Vec<(&'static str, String)>>> =
                Arc::new(Mutex::new(Vec::new()));
            let pb_for_task = pb.clone();
            let multi_clone = self.multi.clone();

            let handle = tokio::spawn(async move {
                let start_time = Instant::now();
                let result = Self::run_streaming_command(
                    task,
                    pb_for_task.clone(),
                    Arc::clone(&output_buffer),
                    &global_options,
                    &multi_clone,
                )
                .await;

                let elapsed = start_time.elapsed();

                match result {
                    Ok(_) => {
                        // Success: update progress bar temporarily
                        pb_for_task.set_style(
                            ProgressStyle::with_template(&format!("  {} {{msg}}", SUCCESS))
                                .unwrap(),
                        );
                        let timing_msg = if global_options.verbose {
                            format!("{} ({:.2}s)", task_name, elapsed.as_secs_f64())
                        } else {
                            task_name.clone()
                        };
                        pb_for_task.set_message(timing_msg);
                        pb_for_task.finish();
                        Ok(())
                    }
                    Err(e) => {
                        // Failure: update progress bar temporarily
                        pb_for_task.set_style(
                            ProgressStyle::with_template(&format!("  {} {{msg}}", FAILURE))
                                .unwrap(),
                        );
                        let timing_msg = if global_options.verbose {
                            format!("{} - FAILED ({:.2}s)", task_name, elapsed.as_secs_f64())
                        } else {
                            format!("{} - FAILED", task_name)
                        };
                        pb_for_task.set_message(timing_msg);
                        pb_for_task.finish();

                        // Store failed task info for error output
                        {
                            let mut failed = failed_tasks.lock().unwrap();
                            let output_clone = output_buffer.lock().unwrap().clone();
                            failed.push((task_name.clone(), output_clone));
                        }

                        Err(e)
                    }
                }
            });

            handles.push((handle, task_name_for_handle));
        }

        // Wait for all tasks to complete
        for (handle, task_name) in handles {
            match handle.await {
                Ok(Ok(_)) => {
                    // Task completed successfully
                }
                Ok(Err(_)) => {
                    // Task failed - output already displayed immediately when it failed
                }
                Err(e) => {
                    // For panicked tasks, create a synthetic stderr entry and display immediately
                    let panic_output = vec![("stderr", format!("Task panicked: {e}"))];
                    self.multi.suspend(|| {
                        eprintln!("{}", format_command_output(&task_name, &panic_output));
                    });

                    let mut failed = failed_tasks.lock().unwrap();
                    failed.push((task_name, panic_output));
                }
            }
        }

        // Print error output for failed tasks
        let failed = failed_tasks.lock().unwrap();
        for (task_name, output) in failed.iter() {
            if output.is_empty() {
                eprintln!("Command failed with no output");
            } else {
                eprintln!("{}", format_command_output(task_name, output));
            }
        }

        // Return error if any tasks failed
        if failed.is_empty() {
            Ok(())
        } else {
            eprintln!(
                "Error: {} - {} of {} tasks failed",
                self.main_message,
                failed.len(),
                task_count
            );
            anyhow::bail!("{} streaming tasks failed", failed.len())
        }
    }

    async fn run_streaming_command(
        mut task: StreamingTask,
        pb: ProgressBar,
        output_buffer: Arc<Mutex<Vec<(&'static str, String)>>>,
        _global_options: &GlobalOptions,
        _multi: &MultiProgress,
    ) -> Result<()> {
        let mut child = task
            .command
            .spawn()
            .with_context(|| format!("Failed to spawn command for task: {}", task.name))?;

        let stdout = child
            .stdout
            .take()
            .context("Failed to take stdout from child process")?;
        let stderr = child
            .stderr
            .take()
            .context("Failed to take stderr from child process")?;

        // Create async streams for stdout and stderr
        let stdout_reader = BufReader::new(stdout);
        let stderr_reader = BufReader::new(stderr);

        let stdout_lines = LinesStream::new(stdout_reader.lines());
        let stderr_lines = LinesStream::new(stderr_reader.lines());

        // Combine both streams
        let mut combined_stream = Box::pin(tokio_stream::StreamExt::merge(
            stdout_lines.map(|line| ("stdout", line)),
            stderr_lines.map(|line| ("stderr", line)),
        ));

        let mut line_count = 0;

        // Process output lines as they arrive
        while let Some((source, line_result)) = combined_stream.next().await {
            match line_result {
                Ok(line) => {
                    line_count += 1;

                    // Store in output buffer
                    {
                        let mut buffer = output_buffer.lock().unwrap();
                        buffer.push((source, line.clone()));
                    }

                    // Update progress bar with line count and latest line
                    let trimmed_line = line.trim_start();

                    // Get actual terminal width, fallback to 80 if detection fails
                    let terminal_width = terminal::size()
                        .map(|(cols, _)| cols as usize)
                        .unwrap_or(80);

                    // Calculate available space for the line content
                    // Account for the spinner and indentation from indicatif
                    let prefix = format!("{} - {} lines - ", task.name, line_count);
                    let ui_overhead = 4; // Account for spinner "  🔄 " and some padding
                    let available_width = terminal_width
                        .saturating_sub(prefix.len())
                        .saturating_sub(ui_overhead);

                    let latest_line = if trimmed_line.len() > available_width && available_width > 3
                    {
                        format!(
                            "{}...",
                            trimmed_line
                                .chars()
                                .take(available_width.saturating_sub(3))
                                .collect::<String>()
                        )
                    } else if available_width > 3 {
                        trimmed_line.to_string()
                    } else {
                        "...".to_string() // Terminal too narrow
                    };

                    let display_text = format!("{}{}", prefix, latest_line);
                    pb.set_message(display_text);
                }
                Err(e) => {
                    pb.set_message(format!("{} - Read error: {}", task.name, e));
                    break;
                }
            }
        }

        // Wait for the child process to complete
        let status = child
            .wait()
            .await
            .with_context(|| format!("Failed to wait for command completion: {}", task.name))?;

        if status.success() {
            Ok(())
        } else {
            // Don't display failure immediately - let the runner handle it after all tasks complete
            anyhow::bail!("Command failed with exit code {:?}", status.code())
        }
    }

    async fn run_dry_run(&self, tasks: Vec<StreamingTask>) -> Result<()> {
        // Clear the spinner - we'll print static output
        self.main_pb.finish_and_clear();

        println!("{} {}", DRY_RUN, self.main_message);

        for task in tasks {
            println!("  {} {}", DRY_RUN, task.name);
        }

        Ok(())
    }
}

/// Format command output with clear stdout/stderr separation and empty section handling
fn format_command_output(task_name: &str, output: &[(&'static str, String)]) -> String {
    let mut result = String::new();

    // Add task failure header
    result.push_str(&format!("\n--- {} failed ---\n", task_name));

    // Separate stdout and stderr
    let stdout_lines: Vec<&String> = output
        .iter()
        .filter(|(source, _)| *source == "stdout")
        .map(|(_, line)| line)
        .collect();

    let stderr_lines: Vec<&String> = output
        .iter()
        .filter(|(source, _)| *source == "stderr")
        .map(|(_, line)| line)
        .collect();

    // Handle stdout section
    if !stdout_lines.is_empty() {
        result.push_str("[stdout]\n");
        for line in stdout_lines {
            result.push_str(line.trim_start());
            result.push('\n');
        }
        result.push('\n');
    }

    // Handle stderr section
    if !stderr_lines.is_empty() {
        result.push_str("[stderr]\n");
        for line in stderr_lines {
            result.push_str(line.trim_start());
            result.push('\n');
        }
    }

    // Add closing separator
    result.push_str("---\n");

    result
}
