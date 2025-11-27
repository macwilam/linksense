/*
    Code Structure and Execution Analysis for `scheduler.rs`

    This module is responsible for orchestrating the execution of monitoring tasks. It acts as a time-based job scheduler, ensuring that tasks like 'ping' or 'traceroute' are run at their configured intervals.

    **Core Components (Structs):**

    1.  `TaskScheduler`: This is the central struct of the module. It holds the overall state and logic for managing all tasks.
        -   `tasks_config`: A shared, thread-safe reference (`Arc<RwLock<>>`) to the current configuration of all tasks. This allows the configuration to be updated dynamically without stopping the scheduler.
        -   `database`: A shared, thread-safe handle to the agent's database, used for persisting task results (metrics).
        -   `task_executor`: An instance of `TaskExecutor` (defined in `tasks.rs`), which is responsible for the *actual* execution of a task's logic. The scheduler handles the "when," and the executor handles the "what."
        -   `result_receiver` / `result_sender`: A multi-producer, single-consumer (MPSC) channel. The `TaskExecutor` uses the `sender` to send back results of completed tasks. The `TaskScheduler`'s main loop listens on the `receiver` to process these results.
        -   `running_tasks`: A `HashMap` that tracks the state of each individual task managed by the scheduler. The key is the task name, and the value is a `TaskHandle`.
        -   `state`: An enum `SchedulerState` that represents the current operational state of the scheduler (e.g., `Running`, `Stopped`).

    2.  `TaskHandle`: Represents a single scheduled task. It's an internal bookkeeping struct for the `TaskScheduler`.
        -   `name`: The unique identifier for the task.
        -   `config`: A copy of the task's specific configuration (`TaskConfig`).
        -   `interval`: A `tokio::time::Interval` that fires whenever the task is due to be run, based on its schedule.
        -   `is_running`: A boolean flag to prevent task overruns. If a task is still running when its next scheduled time arrives, the new execution is skipped.

    3.  `SchedulerState`: A simple enum (`Stopped`, `Starting`, `Running`, `Stopping`) to manage the lifecycle of the scheduler in a clear and predictable way.

    4.  `SchedulerStats`: A data structure for exposing monitoring information about the scheduler's performance, such as the number of running tasks and success/failure counts.

    **Execution Flow and Method Dependencies:**

    The intended lifecycle and flow of control are as follows:

    1.  **Initialization**:
        -   `TaskScheduler::new(config, database)` is the entry point. It's called once when the agent starts.
        -   It creates the MPSC channel for results and initializes the `TaskExecutor`.
        -   The scheduler starts in the `SchedulerState::Stopped` state.

    2.  **Starting the Scheduler**:
        -   `scheduler.start().await` is called.
        -   This method transitions the state to `SchedulerState::Starting`.
        -   It reads the initial task configurations and creates a `TaskHandle` for each one, populating the `running_tasks` map.
        -   Finally, it sets the state to `SchedulerState::Running`.

    3.  **Main Operational Loop**:
        -   `scheduler.run().await` is called immediately after `start()`. This method contains the main loop and will run until the scheduler is stopped.
        -   The loop's behavior is driven by the `state` machine.
        -   When `state` is `Running`, it repeatedly calls `process_scheduler_tick().await`.

    4.  **A Single "Tick" of the Scheduler (`process_scheduler_tick`)**:
        -   **Process Results**: It first checks the `result_receiver` for any completed task results using a non-blocking `try_recv`. For each result, it calls `handle_task_result`.
        -   **Check for Due Tasks**: It then calls `get_tasks_ready_to_run` to get a list of tasks whose `interval` has elapsed.
        -   **Execute Tasks**: For each ready task, it calls `execute_single_task`.

    5.  **Task Execution (`execute_single_task`)**:
        -   It finds the corresponding `TaskHandle` and marks `is_running = true`.
        -   Crucially, it **spawns a new asynchronous Tokio task** to perform the actual work. This is vital to prevent a single long-running task from blocking the entire scheduler loop.
        -   Inside this new task, it would typically call `self.task_executor.execute(...)` (currently a placeholder `tokio::time::sleep` is used).
        -   The result of the execution is sent back to the scheduler via the `result_sender`.

    6.  **Handling Results (`handle_task_result`)**:
        -   This method is called when a result is received from the channel.
        -   It finds the `TaskHandle` and resets `is_running = false`.
        -   If the task was successful and produced metrics (`MetricData`), it acquires a lock on the `database` and calls `store_raw_metric`.
        -   It logs the outcome of the task.

    7.  **Dynamic Updates**:
        -   `scheduler.update_tasks(new_config).await` can be called at any time to dynamically reconfigure the agent's tasks. It clears the old tasks, updates the central `tasks_config`, and rebuilds the `running_tasks` map from the new configuration.

    8.  **Stopping**:
        -   `scheduler.stop().await` transitions the state to `Stopping`, clears all `running_tasks`, and finally sets the state to `Stopped`, which causes the `run()` loop to exit.

    **Summary of Responsibilities:**

    -   `TaskScheduler`: Manages the "when" (scheduling) and orchestrates the overall flow.
    -   `TaskExecutor`: Manages the "what" (the actual logic of running a ping, etc.).
    -   `TaskHandle`: Internal state for a single task within the scheduler.
    -   `AgentDatabase`: Handles the "where" (storage of results).
*/
//! Task scheduling and execution for the network monitoring agent
//!
//! This module manages the execution of monitoring tasks based on their
//! configured schedules. It handles concurrent task execution, state management,
//! and coordination with the database for storing results.
// The scheduler is the heart of the agent, responsible for the "when" and "how"
// of task execution. It's designed to be robust and handle a dynamic set of
// tasks that can be updated at runtime.

use anyhow::Result;
use shared::config::{TaskConfig, TaskType, TasksConfig};
use shared::metrics::MetricData;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::{mpsc, RwLock};
use tokio::time::{Duration, Instant};
use tracing::{debug, info, warn};

use crate::database::AgentDatabase;
use crate::tasks::TaskExecutor;

/// Manages the scheduling and execution of all monitoring tasks.
pub struct TaskScheduler {
    /// The current task configuration, wrapped in `Arc<RwLock<>>` to allow
    /// safe, shared access and modification from multiple async tasks.
    tasks_config: Arc<RwLock<TasksConfig>>,
    /// A shared reference to the database for storing task results.
    pub database: Arc<RwLock<AgentDatabase>>,
    /// The component responsible for actually executing the logic of each task.
    task_executor: TaskExecutor,
    /// A channel receiver for collecting results from completed tasks.
    /// The `TaskExecutor` sends results to the `result_sender`.
    pub result_receiver: mpsc::Receiver<crate::tasks::TaskResult>,
    /// The sender part of the channel for task results. It's cloned and given
    /// to the `TaskExecutor`.
    #[allow(dead_code)]
    result_sender: mpsc::Sender<crate::tasks::TaskResult>,
    /// A channel receiver for getting notifications that a task is ready to run.
    pub ready_receiver: mpsc::Receiver<String>,
    /// The sender part of the channel for ready notifications. It's cloned
    /// and given to each spawned ticker task.
    ready_sender: mpsc::Sender<String>,
    /// A map to keep track of the state of each scheduled task, including its
    /// timer and whether it's currently running.
    running_tasks: HashMap<String, TaskHandle>,
    /// The overall state of the scheduler (e.g., Running, Stopped).
    pub state: SchedulerState,
    /// The last time aggregation was performed (as Unix timestamp)
    pub last_aggregation: u64,
    /// Buffer for metrics waiting to be written to database
    pub metrics_buffer: Vec<MetricData>,
    /// The last time metrics were written to database (as Unix timestamp)
    pub last_db_write: u64,
    /// Interval in seconds between database flushes for buffered metrics
    pub flush_interval_seconds: u64,
    /// Maximum size of the metrics buffer before forcing a flush (prevents unbounded memory growth)
    pub max_metrics_buffer_size: usize,
    /// Maximum time in seconds to wait for in-flight tasks during graceful shutdown
    pub graceful_shutdown_timeout_secs: u64,
    /// Channel buffer size for task results
    #[allow(dead_code)]
    pub channel_buffer_size: usize,
    /// The last time queue cleanup was performed (as Unix timestamp)
    pub last_queue_cleanup: u64,
    /// Interval in seconds between queue cleanup operations
    pub queue_cleanup_interval_seconds: u64,
}

/// Represents a handle to an individual scheduled task.
/// It holds the task's configuration and its execution state.
struct TaskHandle {
    /// The unique name of the task.
    name: String,
    /// A copy of the task's configuration.
    config: TaskConfig,
    /// A flag to indicate if the task is currently executing. This is used
    /// to prevent a task from being started again if its previous run hasn't
    /// finished yet (a condition known as "overrun").
    is_running: bool,
    /// The handle to the spawned ticker task that sends notifications on the
    /// `ready_sender` channel when the task's interval fires.
    join_handle: tokio::task::JoinHandle<()>,
}

/// Represents the possible states of the scheduler.
/// Using an enum for state management makes the logic clearer and less error-prone.
#[derive(Debug, Clone, PartialEq)]
pub enum SchedulerState {
    Stopped,
    Running,
}

impl TaskScheduler {
    /// Creates a new `TaskScheduler`.
    ///
    /// # Parameters
    /// * `tasks_config` - Initial task configuration
    /// * `database` - Shared database handle for storing metrics
    /// * `flush_interval_seconds` - Interval in seconds between database flushes
    /// * `graceful_shutdown_timeout_secs` - Maximum time to wait for in-flight tasks during shutdown
    /// * `channel_buffer_size` - Size of MPSC channel buffers for task communication
    /// * `queue_cleanup_interval_seconds` - Interval in seconds between queue cleanup operations
    /// * `server_url` - Optional server URL for bandwidth tests
    /// * `api_key` - Optional API key for server authentication
    /// * `agent_id` - Optional agent ID for identification
    ///
    /// # Returns
    /// `TaskScheduler` instance or error if initialization fails
    pub fn new(
        tasks_config: TasksConfig,
        database: Arc<RwLock<AgentDatabase>>,
        flush_interval_seconds: u32,
        graceful_shutdown_timeout_secs: u64,
        channel_buffer_size: usize,
        queue_cleanup_interval_seconds: u64,
        server_url: Option<String>,
        api_key: Option<String>,
        agent_id: Option<String>,
    ) -> Result<Self> {
        // A MPSC (multi-producer, single-consumer) channel is used to communicate
        // task results from the executor back to the scheduler's main loop.
        let (result_sender, result_receiver) = mpsc::channel(channel_buffer_size);
        // A second MPSC channel is used for ticker tasks to notify the scheduler
        // that a task is ready to be executed.
        let (ready_sender, ready_receiver) = mpsc::channel(channel_buffer_size);
        let task_executor =
            TaskExecutor::new(result_sender.clone(), server_url, api_key, agent_id)?;

        // Calculate max buffer size: For high-frequency monitoring, we want to buffer
        // enough metrics to avoid frequent DB writes, but not so many that we risk OOM.
        // Default to 10,000 metrics (approximately 1-2 MB depending on metric type).
        // This allows for ~166 tasks running every second for a full minute before forcing flush.
        let max_metrics_buffer_size = 10_000;

        Ok(Self {
            tasks_config: Arc::new(RwLock::new(tasks_config)),
            database,
            task_executor,
            result_receiver,
            result_sender,
            ready_receiver,
            ready_sender,
            running_tasks: HashMap::new(),
            state: SchedulerState::Stopped,
            last_aggregation: 0,
            metrics_buffer: Vec::new(),
            last_db_write: 0,
            flush_interval_seconds: flush_interval_seconds as u64,
            max_metrics_buffer_size,
            graceful_shutdown_timeout_secs,
            channel_buffer_size,
            last_queue_cleanup: 0,
            queue_cleanup_interval_seconds,
        })
    }

    /// Starts the scheduler.
    ///
    /// This initializes and spawns the ticker tasks for each configured task.
    /// The scheduler state is set to Running after all tasks are initialized.
    ///
    /// # Returns
    /// `Ok(())` on success, error if initialization fails
    pub async fn start(&mut self) -> Result<()> {
        info!("Starting task scheduler");

        let tasks_to_start = {
            let config = self.tasks_config.read().await;
            config.tasks.clone()
        };

        let min_delay = Self::calculate_minimum_start_delay(&tasks_to_start);
        info!(
            "Calculated minimum delay between task starts: {:?}",
            min_delay
        );
        let mut current_delay = Duration::from_millis(0);

        for task_config in &tasks_to_start {
            self.spawn_ticker_task(task_config, current_delay);
            current_delay += min_delay;
        }

        self.state = SchedulerState::Running;
        info!(
            "Task scheduler started with {} tasks",
            self.running_tasks.len()
        );

        Ok(())
    }

    /// Spawns a dedicated ticker task for a given monitoring task.
    ///
    /// This ticker will send a notification on the `ready_sender` channel
    /// every time the task's interval fires.
    ///
    /// # Parameters
    /// * `task_config` - The configuration for the task to schedule
    /// * `start_delay` - Initial delay before the first tick
    fn spawn_ticker_task(&mut self, task_config: &TaskConfig, start_delay: Duration) {
        let start_time = Instant::now() + start_delay;
        let mut interval = tokio::time::interval_at(start_time, task_config.schedule_duration());
        let task_name = task_config.name.clone();
        let ready_sender = self.ready_sender.clone();

        let join_handle = tokio::spawn(async move {
            loop {
                interval.tick().await;
                if ready_sender.send(task_name.clone()).await.is_err() {
                    debug!(
                        "Task ticker for '{}' stopping as channel is closed.",
                        task_name
                    );
                    break;
                }
            }
        });

        let handle = TaskHandle {
            name: task_config.name.clone(),
            config: task_config.clone(),
            is_running: false,
            join_handle,
        };
        self.running_tasks.insert(task_config.name.clone(), handle);
    }

    /// Stops the scheduler gracefully.
    ///
    /// This method performs a graceful shutdown:
    /// 1. Sets state to Stopped (signals main loop to exit)
    /// 2. Waits for in-flight tasks to complete (with timeout)
    /// 3. Flushes any buffered metrics to database
    /// 4. Aborts ticker tasks
    ///
    /// # Returns
    /// `Ok(())` on success, error if metrics flush fails
    pub async fn stop(&mut self) -> Result<()> {
        info!("Stopping task scheduler gracefully");
        self.state = SchedulerState::Stopped;

        // Wait for in-flight tasks to complete with timeout
        let in_flight_count = self.running_tasks.values().filter(|h| h.is_running).count();
        if in_flight_count > 0 {
            info!(
                "Waiting for {} in-flight tasks to complete (timeout: {}s)",
                in_flight_count, self.graceful_shutdown_timeout_secs
            );

            let deadline =
                Instant::now() + Duration::from_secs(self.graceful_shutdown_timeout_secs);
            let mut check_interval = tokio::time::interval(Duration::from_millis(100));

            loop {
                check_interval.tick().await;

                // Process any pending results
                while let Ok(result) = self.result_receiver.try_recv() {
                    if let Err(e) = self.handle_task_result(result).await {
                        warn!("Error handling task result during shutdown: {}", e);
                    }
                }

                let still_running = self.running_tasks.values().filter(|h| h.is_running).count();
                if still_running == 0 {
                    info!("All in-flight tasks completed successfully");
                    break;
                }

                if Instant::now() >= deadline {
                    warn!(
                        "Graceful shutdown timeout reached, {} tasks still running",
                        still_running
                    );
                    break;
                }
            }
        }

        // Flush any remaining buffered metrics
        if !self.metrics_buffer.is_empty() {
            info!(
                "Flushing {} buffered metrics before shutdown",
                self.metrics_buffer.len()
            );
            if let Err(e) = self.flush_metrics_now().await {
                warn!("Failed to flush metrics during shutdown: {}", e);
            }
        }

        // Abort all ticker tasks
        self.clear_and_abort_tasks();

        info!("Task scheduler stopped");

        Ok(())
    }

    /// Aborts all running ticker tasks and clears the `running_tasks` map.
    ///
    /// This is called during scheduler shutdown to ensure all spawned tasks
    /// are properly terminated.
    fn clear_and_abort_tasks(&mut self) {
        let task_count = self.running_tasks.len();
        for (_, handle) in self.running_tasks.drain() {
            handle.join_handle.abort();
        }
        debug!("Aborted {} ticker tasks", task_count);
    }

    /// Executes a single task by its name.
    ///
    /// Checks if the task is already running to prevent overruns, then spawns
    /// a new async task to execute it without blocking the scheduler loop.
    ///
    /// # Parameters
    /// * `task_name` - The name of the task to execute
    ///
    /// # Returns
    /// `Ok(())` on successful spawn, or an error if something goes wrong
    pub async fn execute_single_task(&mut self, task_name: &str) -> Result<()> {
        if let Some(handle) = self.running_tasks.get_mut(task_name) {
            if handle.is_running {
                // If the task is already running, skip this execution.
                // This prevents task overruns.
                warn!(
                    "Skipping execution of task '{}' as it is already running.",
                    handle.name
                );
                return Ok(());
            }

            debug!("Executing task: {}", handle.name);
            handle.is_running = true;

            // The actual task execution is spawned onto a new tokio task.
            // This allows the scheduler loop to remain responsive and not get
            // blocked by a long-running task.
            let task_executor = self.task_executor.clone();
            let task_config = handle.config.clone();

            tokio::spawn(async move {
                if let Err(e) = task_executor.execute_task(&task_config).await {
                    tracing::error!("Task execution failed: {}", e);
                }
            });
        }

        Ok(())
    }

    /// Handles the result of a completed task.
    ///
    /// Updates task state, buffers metrics for database storage, and logs
    /// the outcome. If the buffer exceeds the maximum size, forces an immediate flush.
    ///
    /// # Parameters
    /// * `result` - The result from the completed task execution
    ///
    /// # Returns
    /// `Ok(())` on successful handling
    pub async fn handle_task_result(&mut self, result: crate::tasks::TaskResult) -> Result<()> {
        debug!("Received task result for: {}", result.task_name);

        // Update the task's state to indicate it's no longer running.
        if let Some(handle) = self.running_tasks.get_mut(&result.task_name) {
            handle.is_running = false;
        }

        // If the task produced metric data, add it to the buffer.
        if let Some(metric_data) = result.metric_data {
            self.metrics_buffer.push(metric_data);

            // Check if buffer has exceeded maximum size and force flush if needed
            if self.metrics_buffer.len() >= self.max_metrics_buffer_size {
                warn!(
                    "Metrics buffer reached maximum size ({}), forcing immediate flush",
                    self.max_metrics_buffer_size
                );
                if let Err(e) = self.flush_metrics_now().await {
                    warn!("Failed to flush metrics buffer on size limit: {}", e);
                    // If flush fails, prevent unbounded growth by removing oldest metrics
                    // Keep only the most recent 50% of metrics
                    let keep_size = self.max_metrics_buffer_size / 2;
                    if self.metrics_buffer.len() > keep_size {
                        warn!(
                            "Emergency buffer cleanup: removing {} oldest metrics",
                            self.metrics_buffer.len() - keep_size
                        );
                        self.metrics_buffer
                            .drain(0..self.metrics_buffer.len() - keep_size);
                    }
                }
            }
        }

        // Log the outcome of the task.
        if result.success {
            debug!(
                "Task '{}' completed successfully in {:.1}ms",
                result.task_name, result.execution_time_ms
            );
        } else {
            warn!(
                "Task '{}' failed: {}",
                result.task_name,
                result.error.unwrap_or_else(|| "Unknown error".to_string())
            );
        }

        Ok(())
    }

    /// Calculates a minimum delay to stagger task starts.
    ///
    /// This helps to avoid a "thundering herd" problem where all tasks
    /// start at the exact same time, especially after a restart or
    /// configuration update.
    ///
    /// The delay is calculated based on the expected task execution rate,
    /// then divided by 2 to balance startup time with load distribution.
    ///
    /// # Parameters
    /// * `tasks` - The list of task configurations to analyze
    ///
    /// # Returns
    /// Duration representing the minimum delay between task starts
    fn calculate_minimum_start_delay(tasks: &[TaskConfig]) -> Duration {
        // Calculate the total number of task executions expected per minute.
        let total_executions_per_minute: f64 = tasks
            .iter()
            .map(|task| 60.0 / task.schedule_seconds as f64)
            .sum();

        if total_executions_per_minute < 1.0 {
            // If less than one execution per minute, no delay is needed.
            return Duration::from_millis(0);
        }

        // Calculate the average time between task executions.
        let average_delay_seconds = 60.0 / total_executions_per_minute;
        debug!(
            "Calculated task scheduling metrics: {:.2} executions/min, avg delay: {:.3}s",
            total_executions_per_minute, average_delay_seconds
        );

        // Divide by 2 to provide staggering without excessive startup delay
        let minimum_delay_seconds = average_delay_seconds / 2.0;

        Duration::from_secs_f64(minimum_delay_seconds)
    }

    /// Flushes buffered metrics to database if enough time has passed.
    ///
    /// Checks if the configured flush interval has elapsed since the last write
    /// and if there are metrics in the buffer. If both conditions are met,
    /// writes all buffered metrics to the database.
    ///
    /// # Returns
    /// `Ok(())` on success or if no flush needed, database error otherwise
    pub async fn flush_metrics_if_needed(&mut self) -> Result<()> {
        let current_time = self.get_current_timestamp();

        // Only flush if the configured interval has passed and we have metrics to write
        if current_time >= self.last_db_write + self.flush_interval_seconds
            && !self.metrics_buffer.is_empty()
        {
            let metrics_to_write = std::mem::take(&mut self.metrics_buffer);
            let metrics_count = metrics_to_write.len();

            debug!("Flushing {} buffered metrics to database", metrics_count);

            let mut db = self.database.write().await;
            for metric in metrics_to_write {
                db.store_raw_metric(&metric).await?;
            }

            self.last_db_write = current_time;
            debug!("Successfully flushed {} metrics to database", metrics_count);
        }

        Ok(())
    }

    /// Forces a flush of all buffered metrics to database regardless of timing.
    ///
    /// This is called before aggregation to ensure all raw metrics are
    /// persisted before the aggregation process begins.
    ///
    /// # Returns
    /// `Ok(())` on success or if buffer empty, database error otherwise
    pub async fn flush_metrics_now(&mut self) -> Result<()> {
        if !self.metrics_buffer.is_empty() {
            let metrics_to_write = std::mem::take(&mut self.metrics_buffer);
            let metrics_count = metrics_to_write.len();

            debug!(
                "Force flushing {} buffered metrics to database before aggregation",
                metrics_count
            );

            let mut db = self.database.write().await;
            for metric in metrics_to_write {
                db.store_raw_metric(&metric).await?;
            }

            self.last_db_write = self.get_current_timestamp();
            debug!(
                "Successfully force flushed {} metrics to database",
                metrics_count
            );
        }

        Ok(())
    }

    /// Checks if aggregation should be performed and does it if needed.
    ///
    /// Aggregation occurs once per minute boundary. When a new minute is
    /// detected, this method flushes any buffered metrics and then aggregates
    /// all raw metrics from the previous minute for each configured task.
    ///
    /// # Returns
    /// `Ok(())` on success, error if aggregation or database operations fail
    pub async fn check_and_perform_aggregation(&mut self) -> Result<()> {
        let current_time = self.get_current_timestamp();
        // Round down to full minute using saturating arithmetic to prevent overflow
        let current_minute = current_time.saturating_div(60).saturating_mul(60);
        debug!("Checking aggregator");
        // Check if we've moved to a new minute
        if current_minute > self.last_aggregation {
            // Flush any remaining buffered metrics before aggregation
            self.flush_metrics_now().await?;

            let period_end = current_minute;
            let period_start = period_end.saturating_sub(60); // Previous minute

            debug!(
                "Performing aggregation for period {}-{}",
                period_start, period_end
            );

            // Get all unique task names and types from our configuration
            let task_configs = {
                let config = self.tasks_config.read().await;
                config.tasks.clone()
            };

            // Perform aggregation for each task
            for task_config in task_configs {
                self.aggregate_task_metrics(
                    &task_config.name,
                    &task_config.task_type,
                    period_start,
                    period_end,
                )
                .await?;
            }

            self.last_aggregation = current_minute;
            info!(
                "Completed aggregation for period {}-{}",
                period_start, period_end
            );

            // Perform WAL checkpoint after aggregation to merge changes and keep WAL small
            debug!("Performing WAL checkpoint after aggregation");
            let mut db = self.database.write().await;
            match db.checkpoint_wal().await {
                Ok(frames) => {
                    debug!("WAL checkpoint complete: {} frames checkpointed", frames);
                }
                Err(e) => {
                    warn!("Failed to checkpoint WAL: {}", e);
                }
            }
        }

        Ok(())
    }

    /// Performs aggregation for a specific task and stores the result.
    ///
    /// Queries the database for raw metrics within the specified time period,
    /// generates aggregated statistics, stores them, and optionally sends
    /// them to the metrics channel for transmission to the server.
    ///
    /// # Parameters
    /// * `task_name` - Name of the task to aggregate
    /// * `task_type` - Type of the task (affects aggregation logic)
    /// * `period_start` - Start of the aggregation period (Unix timestamp)
    /// * `period_end` - End of the aggregation period (Unix timestamp)
    ///
    /// # Returns
    /// `Ok(())` on success, error if database operations fail
    async fn aggregate_task_metrics(
        &self,
        task_name: &str,
        task_type: &TaskType,
        period_start: u64,
        period_end: u64,
    ) -> Result<()> {
        let mut db = self.database.write().await;

        if let Some(aggregated_metrics) = db
            .generate_aggregated_metrics(task_name, task_type, period_start, period_end)
            .await?
        {
            // Store and automatically enqueue for sending
            db.store_and_enqueue_aggregated_metrics(&aggregated_metrics)
                .await?;
            debug!(
                "Stored and enqueued aggregated metrics for task '{}' (type: {:?})",
                task_name, task_type
            );
        } else {
            debug!(
                "No raw metrics found for task '{}' in period {}-{}",
                task_name, period_start, period_end
            );
        }

        Ok(())
    }

    /// Gets the current Unix timestamp.
    ///
    /// # Returns
    /// Current time as seconds since Unix epoch
    pub fn get_current_timestamp(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }

    /// Checks if the scheduler is currently in the `Running` state.
    #[allow(dead_code)]
    pub fn is_running(&self) -> bool {
        self.state == SchedulerState::Running
    }

    /// Cleans up old sent queue entries if enough time has passed.
    ///
    /// Checks if the configured cleanup interval has elapsed since the last cleanup.
    /// If so, removes queue entries that have been successfully sent and are older
    /// than 24 hours.
    ///
    /// # Returns
    /// `Ok(())` on success or if no cleanup needed, database error otherwise
    pub async fn cleanup_sent_queue_if_needed(&mut self) -> Result<()> {
        let current_time = self.get_current_timestamp();

        // Only cleanup if the configured interval has passed
        if current_time >= self.last_queue_cleanup + self.queue_cleanup_interval_seconds {
            debug!("Performing queue cleanup");

            let mut db = self.database.write().await;
            if let Err(e) = db.cleanup_sent_queue_entries(24).await {
                warn!("Failed to cleanup sent queue entries: {}", e);
            } else {
                debug!("Cleaned up old sent queue entries");
            }

            self.last_queue_cleanup = current_time;
        }

        Ok(())
    }

    /// Refreshes the HTTP clients and TLS connectors in the task executor
    ///
    /// This method calls refresh_clients() on the task executor to drop and recreate
    /// HTTP clients and TLS connectors. This helps prevent memory leaks and ensures
    /// clean state for HTTP/TLS operations.
    ///
    /// # Returns
    /// `Ok(())` on successful refresh, error if client/connector creation fails
    pub fn refresh_http_clients(&mut self) -> Result<()> {
        self.task_executor.refresh_clients()
    }
}
