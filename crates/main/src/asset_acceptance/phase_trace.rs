//! Opt-in wall-time capture for Bevy schedule and selected render spans.
//!
//! This is profiling evidence only. The `asset-review-schedule-trace` feature
//! enables Bevy's schedule spans; the custom layer is installed only when
//! `TERRA_PLANET_SCHEDULE_TRACE` is set.

use bevy::{
    app::App,
    ecs::prelude::Resource,
    log::{
        BoxedLayer,
        tracing::{self, Subscriber},
        tracing_subscriber::{self, Layer},
    },
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

pub(super) const ENV: &str = "TERRA_PLANET_SCHEDULE_TRACE";
const NATIVE_SYSTEM_MIN_DURATION: Duration = Duration::from_millis(1);
const NATIVE_SYSTEM_ROW_LIMIT: usize = 50_000;

#[derive(Clone, Resource)]
pub(super) struct ScheduleTraceRecorder {
    shared: Arc<Mutex<ScheduleTraceState>>,
}

struct ScheduleTraceState {
    active: Option<Measurement>,
    labels: HashMap<tracing::span::Id, TraceSpanLabel>,
    open: HashMap<tracing::span::Id, OpenSchedule>,
    rows: Vec<ScheduleTraceRow>,
    frame_clocks: Vec<FrameClockSample>,
    native_system_profile: NativeSystemProfile,
    native_system_counts: HashMap<(&'static str, u8), NativeSystemCounts>,
}

#[derive(Clone)]
struct Measurement {
    route: &'static str,
    repeat: u8,
    started: Instant,
}

struct OpenSchedule {
    label: String,
    profiled_system: bool,
    entered: Instant,
    thread: std::thread::ThreadId,
    measurement: Option<Measurement>,
}

#[derive(Clone)]
struct TraceSpanLabel {
    label: String,
    profiled_system: bool,
}

#[derive(Clone, Copy)]
struct NativeSystemProfile {
    minimum_duration: Duration,
    row_limit: usize,
}

impl Default for NativeSystemProfile {
    fn default() -> Self {
        Self {
            minimum_duration: NATIVE_SYSTEM_MIN_DURATION,
            row_limit: NATIVE_SYSTEM_ROW_LIMIT,
        }
    }
}

#[derive(Default)]
struct NativeSystemCounts {
    retained: usize,
    dropped: usize,
}

struct ScheduleTraceRow {
    route: &'static str,
    repeat: u8,
    span: String,
    route_elapsed_start_s: f64,
    route_elapsed_end_s: f64,
    duration_ms: f64,
    thread: String,
}

struct FrameClockSample {
    route: &'static str,
    repeat: u8,
    real_elapsed_s: f64,
    wall_elapsed_s: f64,
    time_interval_start_s: Option<f64>,
    time_interval_end_s: Option<f64>,
}

impl Default for ScheduleTraceRecorder {
    fn default() -> Self {
        Self {
            shared: Arc::new(Mutex::new(ScheduleTraceState {
                active: None,
                labels: HashMap::new(),
                open: HashMap::new(),
                rows: Vec::new(),
                frame_clocks: Vec::new(),
                native_system_profile: NativeSystemProfile::default(),
                native_system_counts: HashMap::new(),
            })),
        }
    }
}

impl ScheduleTraceRecorder {
    pub(super) fn start_repeat(&self, route: &'static str, repeat: u8) {
        let mut state = self.lock();
        state.active = Some(Measurement {
            route,
            repeat,
            started: Instant::now(),
        });
    }

    pub(super) fn stop_repeat(&self, route: &'static str, repeat: u8) {
        let mut state = self.lock();
        let stopped = Instant::now();
        let Some(measurement) = state
            .active
            .clone()
            .filter(|active| active.route == route && active.repeat == repeat)
        else {
            return;
        };
        let open_ids = state
            .open
            .iter()
            .filter_map(|(id, open)| {
                open.measurement
                    .as_ref()
                    .is_none_or(|open_measurement| {
                        open_measurement.route == route && open_measurement.repeat == repeat
                    })
                    .then(|| id.clone())
            })
            .collect::<Vec<_>>();
        for id in open_ids {
            let Some(open) = state.open.remove(&id) else {
                continue;
            };
            let span_measurement = open
                .measurement
                .clone()
                .unwrap_or_else(|| measurement.clone());
            record_completed_span(&mut state, open, span_measurement, stopped);
        }
        state.active = None;
    }

    pub(super) fn record_frame_sample(
        &self,
        real_elapsed_s: f64,
        last_update: Option<Instant>,
        delta: Duration,
    ) {
        let wall_now = Instant::now();
        let mut state = self.lock();
        let Some(active) = state.active.as_ref() else {
            return;
        };
        let route = active.route;
        let repeat = active.repeat;
        let started = active.started;
        let interval =
            last_update.map(|last_update| frame_interval_offsets(started, last_update, delta));
        state.frame_clocks.push(FrameClockSample {
            route,
            repeat,
            real_elapsed_s,
            wall_elapsed_s: wall_now.duration_since(started).as_secs_f64(),
            time_interval_start_s: interval.map(|(start, _)| start),
            time_interval_end_s: interval.map(|(_, end)| end),
        });
    }

    pub(super) fn take_repeat_csv(&self, route: &'static str, repeat: u8) -> String {
        let mut state = self.lock();
        let mut rows = drain_matching(&mut state.rows, |row| {
            row.route == route && row.repeat == repeat
        });
        rows.sort_by(|left, right| {
            left.route_elapsed_start_s
                .total_cmp(&right.route_elapsed_start_s)
        });

        let mut csv = String::from(
            "route,repeat,span,route_elapsed_start_s,route_elapsed_end_s,duration_ms,thread\n",
        );
        for row in rows {
            csv.push_str(&format!(
                "{},{},{},{:.6},{:.6},{:.6},{}\n",
                csv_field(row.route),
                row.repeat,
                csv_field(&row.span),
                row.route_elapsed_start_s,
                row.route_elapsed_end_s,
                row.duration_ms,
                csv_field(&row.thread),
            ));
        }
        csv
    }

    pub(super) fn take_frame_clock_csv(&self, route: &'static str, repeat: u8) -> String {
        let mut state = self.lock();
        let rows = drain_matching(&mut state.frame_clocks, |sample| {
            sample.route == route && sample.repeat == repeat
        });
        let mut csv = String::from(
            "route,repeat,real_elapsed_s,wall_elapsed_s,time_interval_start_s,time_interval_end_s\n",
        );
        for sample in rows {
            csv.push_str(&format!(
                "{},{},{:.6},{:.6},{},{}\n",
                csv_field(sample.route),
                sample.repeat,
                sample.real_elapsed_s,
                sample.wall_elapsed_s,
                optional_seconds(sample.time_interval_start_s),
                optional_seconds(sample.time_interval_end_s),
            ));
        }
        csv
    }

    pub(super) fn take_native_system_summary_csv(&self, route: &'static str, repeat: u8) -> String {
        let mut state = self.lock();
        let counts = state
            .native_system_counts
            .remove(&(route, repeat))
            .unwrap_or_default();
        format!(
            "route,repeat,threshold_ms,row_cap,profiled_system_rows_retained,profiled_system_rows_dropped\n{},{},{:.3},{},{},{}\n",
            csv_field(route),
            repeat,
            state.native_system_profile.minimum_duration.as_secs_f64() * 1_000.0,
            state.native_system_profile.row_limit,
            counts.retained,
            counts.dropped,
        )
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ScheduleTraceState> {
        self.shared
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

fn relative_seconds(origin: Instant, instant: Instant) -> f64 {
    if instant >= origin {
        instant.duration_since(origin).as_secs_f64()
    } else {
        -origin.duration_since(instant).as_secs_f64()
    }
}

fn frame_interval_offsets(origin: Instant, last_update: Instant, delta: Duration) -> (f64, f64) {
    let end = relative_seconds(origin, last_update);
    (end - delta.as_secs_f64(), end)
}

fn optional_seconds(value: Option<f64>) -> String {
    value.map_or_else(String::new, |seconds| format!("{seconds:.6}"))
}

fn drain_matching<T>(rows: &mut Vec<T>, mut matches: impl FnMut(&T) -> bool) -> Vec<T> {
    let mut remaining = std::mem::take(rows);
    let mut selected = Vec::new();
    let mut retained = Vec::with_capacity(remaining.len());
    for row in remaining.drain(..) {
        if matches(&row) {
            selected.push(row);
        } else {
            retained.push(row);
        }
    }
    *rows = retained;
    selected
}

fn selected_span(metadata: &tracing::Metadata<'_>) -> bool {
    matches!(
        metadata.name(),
        "schedule" | "main_render_schedule" | "present_frames" | "system"
    )
}

fn render_system_label(name: &str) -> Option<&'static str> {
    if name == "prepare_windows" || name.ends_with("::prepare_windows") {
        Some("render-system:prepare_windows")
    } else if name == "process_pipeline_queue_system"
        || name.ends_with("::process_pipeline_queue_system")
    {
        Some("render-system:process_pipeline_queue_system")
    } else {
        None
    }
}

pub(super) fn enabled() -> bool {
    std::env::var_os(ENV).is_some()
}

pub(super) fn install_layer(app: &mut App) -> Option<BoxedLayer> {
    if !enabled() {
        return None;
    }
    let recorder = ScheduleTraceRecorder::default();
    app.insert_resource(recorder.clone());
    Some(Box::new(ScheduleTraceLayer { recorder }.with_filter(
        tracing_subscriber::filter::filter_fn(selected_span),
    )))
}

struct ScheduleTraceLayer {
    recorder: ScheduleTraceRecorder,
}

impl<S: Subscriber> Layer<S> for ScheduleTraceLayer {
    fn on_new_span(
        &self,
        attributes: &tracing::span::Attributes<'_>,
        id: &tracing::span::Id,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let metadata = attributes.metadata();
        let label = match metadata.name() {
            "schedule" => {
                let mut schedule = None;
                attributes.record(&mut ScheduleNameVisitor(&mut schedule));
                schedule.map(|schedule| TraceSpanLabel {
                    label: format!("schedule:{schedule}"),
                    profiled_system: false,
                })
            }
            "main_render_schedule" | "present_frames" => Some(TraceSpanLabel {
                label: format!("render:{}", metadata.name()),
                profiled_system: false,
            }),
            "system" => {
                let mut system_name = None;
                attributes.record(&mut ScheduleNameVisitor(&mut system_name));
                system_name.map(|name| match render_system_label(&name) {
                    Some(label) => TraceSpanLabel {
                        label: label.to_owned(),
                        profiled_system: false,
                    },
                    None => TraceSpanLabel {
                        label: format!("system:{name}"),
                        profiled_system: true,
                    },
                })
            }
            _ => None,
        };
        if let Some(label) = label {
            self.recorder.lock().labels.insert(id.clone(), label);
        }
    }

    fn on_enter(&self, id: &tracing::span::Id, _ctx: tracing_subscriber::layer::Context<'_, S>) {
        let mut state = self.recorder.lock();
        let Some(label) = state.labels.get(id).cloned() else {
            return;
        };
        let measurement = state.active.clone();
        state.open.insert(
            id.clone(),
            OpenSchedule {
                label: label.label,
                profiled_system: label.profiled_system,
                entered: Instant::now(),
                thread: std::thread::current().id(),
                measurement,
            },
        );
    }

    fn on_exit(&self, id: &tracing::span::Id, _ctx: tracing_subscriber::layer::Context<'_, S>) {
        let exited = Instant::now();
        let mut state = self.recorder.lock();
        let Some(open) = state.open.remove(id) else {
            return;
        };
        let Some(measurement) = open.measurement.clone().or_else(|| state.active.clone()) else {
            return;
        };
        record_completed_span(&mut state, open, measurement, exited);
    }

    fn on_close(&self, id: tracing::span::Id, _ctx: tracing_subscriber::layer::Context<'_, S>) {
        let mut state = self.recorder.lock();
        state.labels.remove(&id);
        state.open.remove(&id);
    }
}

fn record_completed_span(
    state: &mut ScheduleTraceState,
    open: OpenSchedule,
    measurement: Measurement,
    exited: Instant,
) {
    let start = open.entered.max(measurement.started);
    if exited <= start {
        return;
    }
    let start_s = start.duration_since(measurement.started).as_secs_f64();
    let end_s = exited.duration_since(measurement.started).as_secs_f64();
    let duration = exited.duration_since(start);
    if open.profiled_system && duration < state.native_system_profile.minimum_duration {
        return;
    }
    if open.profiled_system {
        let row_limit = state.native_system_profile.row_limit;
        let counts = state
            .native_system_counts
            .entry((measurement.route, measurement.repeat))
            .or_default();
        if counts.retained >= row_limit {
            counts.dropped += 1;
            return;
        }
        counts.retained += 1;
    }
    state.rows.push(ScheduleTraceRow {
        route: measurement.route,
        repeat: measurement.repeat,
        span: open.label,
        route_elapsed_start_s: start_s,
        route_elapsed_end_s: end_s,
        duration_ms: duration.as_secs_f64() * 1_000.0,
        thread: format!("{:?}", open.thread),
    });
}

struct ScheduleNameVisitor<'a>(&'a mut Option<String>);

impl tracing::field::Visit for ScheduleNameVisitor<'_> {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "name" {
            *self.0 = Some(value.to_owned());
        }
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "name" {
            *self.0 = Some(format!("{value:?}").trim_matches('"').to_owned());
        }
    }
}

fn csv_field(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::log::tracing_subscriber::prelude::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct EventCounter(Arc<AtomicUsize>);

    impl<S: Subscriber> Layer<S> for EventCounter {
        fn on_event(
            &self,
            _event: &tracing::Event<'_>,
            _ctx: tracing_subscriber::layer::Context<'_, S>,
        ) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[test]
    fn draining_a_repeat_preserves_other_route_samples() {
        let mut rows = vec![
            ("orbit-zoom", 1, "first"),
            ("entry-reversal", 2, "other-route"),
            ("orbit-zoom", 2, "other-repeat"),
            ("orbit-zoom", 1, "second"),
        ];

        let selected = drain_matching(&mut rows, |(route, repeat, _)| {
            *route == "orbit-zoom" && *repeat == 1
        });

        assert_eq!(
            selected,
            [("orbit-zoom", 1, "first"), ("orbit-zoom", 1, "second")]
        );
        assert_eq!(
            rows,
            [
                ("entry-reversal", 2, "other-route"),
                ("orbit-zoom", 2, "other-repeat"),
            ]
        );
    }

    #[test]
    fn records_named_schedule_spans_and_wall_duration() {
        let recorder = ScheduleTraceRecorder::default();
        recorder.start_repeat("orbit-zoom", 2);
        let subscriber = tracing_subscriber::registry().with(
            ScheduleTraceLayer {
                recorder: recorder.clone(),
            }
            .with_filter(tracing_subscriber::filter::filter_fn(selected_span)),
        );

        tracing::subscriber::with_default(subscriber, || {
            let span = tracing::info_span!("schedule", name = ?"PreUpdate");
            let _entered = span.enter();
            std::thread::sleep(std::time::Duration::from_millis(1));
        });

        let csv = recorder.take_repeat_csv("orbit-zoom", 2);
        let row = csv.lines().nth(1).expect("captured schedule span");
        assert!(row.contains("\"schedule:PreUpdate\""));
        let duration_ms = row
            .split(',')
            .nth(5)
            .expect("duration column")
            .parse::<f64>()
            .expect("duration parses");
        assert!(duration_ms >= 1.0, "duration was {duration_ms} ms");
    }

    #[test]
    fn stopping_a_repeat_clips_open_spans_before_drain_and_ignores_late_exit() {
        let recorder = ScheduleTraceRecorder::default();
        recorder.start_repeat("orbit-zoom", 1);
        let repeat_start = recorder
            .lock()
            .active
            .as_ref()
            .expect("active repeat")
            .started;
        let subscriber = tracing_subscriber::registry().with(
            ScheduleTraceLayer {
                recorder: recorder.clone(),
            }
            .with_filter(tracing_subscriber::filter::filter_fn(selected_span)),
        );

        tracing::subscriber::with_default(subscriber, || {
            let span = tracing::info_span!("schedule", name = ?"Render");
            let entered = span.enter();
            std::thread::sleep(Duration::from_millis(2));

            let before_stop = Instant::now();
            recorder.stop_repeat("orbit-zoom", 1);
            let after_stop = Instant::now();
            let csv = recorder.take_repeat_csv("orbit-zoom", 1);
            let row = csv.lines().nth(1).expect("open span captured at stop");
            let fields: Vec<_> = row.split(',').collect();
            let end_s = fields[4].parse::<f64>().expect("span end time parses");
            let stop_start_s = relative_seconds(repeat_start, before_stop);
            let stop_end_s = relative_seconds(repeat_start, after_stop);
            assert!(
                end_s >= stop_start_s - 0.000002 && end_s <= stop_end_s + 0.000002,
                "span end {end_s} must be clipped to stop window [{stop_start_s}, {stop_end_s}]"
            );

            std::thread::sleep(Duration::from_millis(5));
            drop(entered);
            let late_exit_csv = recorder.take_repeat_csv("orbit-zoom", 1);
            assert_eq!(
                late_exit_csv.lines().count(),
                1,
                "late span exit must not append a row after the boundary drain"
            );
        });
    }

    #[test]
    fn stopped_span_keeps_the_thread_that_entered_it() {
        let recorder = ScheduleTraceRecorder::default();
        recorder.start_repeat("orbit-zoom", 1);
        let dispatch = tracing::Dispatch::new(
            tracing_subscriber::registry().with(
                ScheduleTraceLayer {
                    recorder: recorder.clone(),
                }
                .with_filter(tracing_subscriber::filter::filter_fn(selected_span)),
            ),
        );
        let span = tracing::dispatcher::with_default(&dispatch, || {
            tracing::info_span!("system", name = "bevy_render::test_render_system")
        });
        assert_eq!(recorder.lock().labels.len(), 1, "span was selected");
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let worker_span = span.clone();
        let worker_dispatch = dispatch.clone();
        let worker = std::thread::spawn(move || {
            tracing::dispatcher::with_default(&worker_dispatch, || {
                let entered = worker_span.enter();
                entered_tx.send(std::thread::current().id()).unwrap();
                release_rx.recv().unwrap();
                drop(entered);
            });
        });

        let entered_thread = entered_rx.recv().expect("render thread entered span");
        assert_ne!(entered_thread, std::thread::current().id());
        std::thread::sleep(Duration::from_millis(2));
        recorder.stop_repeat("orbit-zoom", 1);
        let csv = recorder.take_repeat_csv("orbit-zoom", 1);
        release_tx.send(()).unwrap();
        worker.join().unwrap();

        let row = csv.lines().nth(1).expect("open render span captured");
        let thread_field = row.split(',').nth(6).expect("thread column");
        assert_eq!(thread_field, csv_field(&format!("{entered_thread:?}")));

        assert_eq!(recorder.take_repeat_csv("orbit-zoom", 1).lines().count(), 1);
    }

    #[test]
    fn schedule_filter_keeps_ordinary_events_visible_to_other_layers() {
        let recorder = ScheduleTraceRecorder::default();
        let sibling_events = Arc::new(AtomicUsize::new(0));
        let subscriber = tracing_subscriber::registry()
            .with(EventCounter(Arc::clone(&sibling_events)))
            .with(
                ScheduleTraceLayer {
                    recorder: recorder.clone(),
                }
                .with_filter(tracing_subscriber::filter::filter_fn(selected_span)),
            );

        tracing::subscriber::with_default(subscriber, || {
            tracing::info!("ordinary application message");
        });

        assert_eq!(sibling_events.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn records_native_render_body_and_present_spans() {
        let recorder = ScheduleTraceRecorder::default();
        recorder.start_repeat("orbit-zoom", 1);
        let subscriber = tracing_subscriber::registry().with(
            ScheduleTraceLayer {
                recorder: recorder.clone(),
            }
            .with_filter(tracing_subscriber::filter::filter_fn(selected_span)),
        );

        tracing::subscriber::with_default(subscriber, || {
            let render = tracing::info_span!("main_render_schedule");
            let _render = render.enter();
            {
                let present = tracing::info_span!("present_frames");
                let _present = present.enter();
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        });

        let csv = recorder.take_repeat_csv("orbit-zoom", 1);
        assert!(csv.contains("\"render:main_render_schedule\""));
        assert!(csv.contains("\"render:present_frames\""));
    }

    #[test]
    fn records_slow_native_system_spans_beyond_the_render_probe() {
        let recorder = ScheduleTraceRecorder::default();
        recorder.start_repeat("orbit-zoom", 1);
        let subscriber = tracing_subscriber::registry().with(
            ScheduleTraceLayer {
                recorder: recorder.clone(),
            }
            .with_filter(tracing_subscriber::filter::filter_fn(selected_span)),
        );

        tracing::subscriber::with_default(subscriber, || {
            let system = tracing::info_span!("system", name = "main::map::update_visibility");
            let _entered = system.enter();
            std::thread::sleep(std::time::Duration::from_millis(3));
        });

        let csv = recorder.take_repeat_csv("orbit-zoom", 1);
        assert!(csv.contains("\"system:main::map::update_visibility\""));
    }

    #[test]
    fn frame_clock_maps_time_real_intervals_to_the_repeat_instant() {
        let recorder = ScheduleTraceRecorder::default();
        recorder.start_repeat("orbit-zoom", 1);
        let repeat_start = recorder
            .lock()
            .active
            .as_ref()
            .expect("active repeat")
            .started;

        recorder.record_frame_sample(
            0.1,
            Some(repeat_start + Duration::from_millis(100)),
            Duration::from_millis(16),
        );

        let csv = recorder.take_frame_clock_csv("orbit-zoom", 1);
        let fields: Vec<_> = csv
            .lines()
            .nth(1)
            .expect("frame sample")
            .split(',')
            .collect();
        let start = fields[4].parse::<f64>().expect("mapped interval start");
        let end = fields[5].parse::<f64>().expect("mapped interval end");
        assert!((start - 0.084).abs() < 1e-6, "start was {start}");
        assert!((end - 0.100).abs() < 1e-6, "end was {end}");
    }

    #[test]
    fn keeps_existing_named_render_probes_and_omits_fast_generic_systems() {
        let recorder = ScheduleTraceRecorder::default();
        recorder.lock().native_system_profile.minimum_duration = Duration::from_secs(1);
        recorder.start_repeat("orbit-zoom", 1);
        let subscriber = tracing_subscriber::registry().with(
            ScheduleTraceLayer {
                recorder: recorder.clone(),
            }
            .with_filter(tracing_subscriber::filter::filter_fn(selected_span)),
        );

        tracing::subscriber::with_default(subscriber, || {
            {
                let prepare = tracing::info_span!(
                    "system",
                    name = "bevy_render::view::window::prepare_windows"
                );
                let _prepare = prepare.enter();
            }
            {
                let pipeline = tracing::info_span!(
                    "system",
                    name = "bevy_render::render_resource::pipeline_cache::PipelineCache::process_pipeline_queue_system"
                );
                let _pipeline = pipeline.enter();
            }
            let ignored = tracing::info_span!("system", name = "unrelated_render_system");
            let _ignored = ignored.enter();
        });

        let csv = recorder.take_repeat_csv("orbit-zoom", 1);
        assert!(csv.contains("\"render-system:prepare_windows\""));
        assert!(csv.contains("\"render-system:process_pipeline_queue_system\""));
        assert!(!csv.contains("unrelated_render_system"));
    }

    #[test]
    fn native_system_profile_caps_rows_and_reports_drops() {
        let recorder = ScheduleTraceRecorder::default();
        recorder.lock().native_system_profile = NativeSystemProfile {
            minimum_duration: Duration::from_millis(5),
            row_limit: 1,
        };
        recorder.start_repeat("orbit-zoom", 2);
        let subscriber = tracing_subscriber::registry().with(
            ScheduleTraceLayer {
                recorder: recorder.clone(),
            }
            .with_filter(tracing_subscriber::filter::filter_fn(selected_span)),
        );

        tracing::subscriber::with_default(subscriber, || {
            let fast = tracing::info_span!("system", name = "main::fast_system");
            let _fast = fast.enter();
            drop(_fast);

            {
                let first = tracing::info_span!("system", name = "main::first_slow_system");
                let _first = first.enter();
                std::thread::sleep(Duration::from_millis(8));
            }
            {
                let second = tracing::info_span!("system", name = "main::second_slow_system");
                let _second = second.enter();
                std::thread::sleep(Duration::from_millis(8));
            }
        });

        let trace_csv = recorder.take_repeat_csv("orbit-zoom", 2);
        let captured_system_rows = trace_csv
            .lines()
            .filter(|line| line.contains(",\"system:"))
            .count();
        assert_eq!(captured_system_rows, 1);
        assert!(trace_csv.contains("first_slow_system"));
        assert!(!trace_csv.contains("fast_system"));
        assert!(!trace_csv.contains("second_slow_system"));

        let summary = recorder.take_native_system_summary_csv("orbit-zoom", 2);
        assert!(
            summary.contains("\"orbit-zoom\",2,5.000,1,1,1"),
            "{summary}"
        );
    }
}
