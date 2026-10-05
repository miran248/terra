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
    time::Instant,
};

pub(super) const ENV: &str = "TERRA_PLANET_SCHEDULE_TRACE";

#[derive(Clone, Resource)]
pub(super) struct ScheduleTraceRecorder {
    shared: Arc<Mutex<ScheduleTraceState>>,
}

struct ScheduleTraceState {
    active: Option<Measurement>,
    labels: HashMap<tracing::span::Id, String>,
    open: HashMap<tracing::span::Id, OpenSchedule>,
    rows: Vec<ScheduleTraceRow>,
    frame_clocks: Vec<FrameClockSample>,
}

#[derive(Clone)]
struct Measurement {
    route: &'static str,
    repeat: u8,
    started: Instant,
}

struct OpenSchedule {
    label: String,
    entered: Instant,
    measurement: Option<Measurement>,
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
        if state
            .active
            .as_ref()
            .is_some_and(|active| active.route == route && active.repeat == repeat)
        {
            state.active = None;
        }
    }

    pub(super) fn record_frame_sample(&self, real_elapsed_s: f64) {
        let wall_now = Instant::now();
        let mut state = self.lock();
        let Some(active) = state.active.as_ref() else {
            return;
        };
        let route = active.route;
        let repeat = active.repeat;
        let started = active.started;
        state.frame_clocks.push(FrameClockSample {
            route,
            repeat,
            real_elapsed_s,
            wall_elapsed_s: wall_now.duration_since(started).as_secs_f64(),
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
        let mut csv = String::from("route,repeat,real_elapsed_s,wall_elapsed_s\n");
        for sample in rows {
            csv.push_str(&format!(
                "{},{},{:.6},{:.6}\n",
                csv_field(sample.route),
                sample.repeat,
                sample.real_elapsed_s,
                sample.wall_elapsed_s,
            ));
        }
        csv
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ScheduleTraceState> {
        self.shared
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
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
                schedule.map(|schedule| format!("schedule:{schedule}"))
            }
            "main_render_schedule" | "present_frames" => {
                Some(format!("render:{}", metadata.name()))
            }
            "system" => {
                let mut system_name = None;
                attributes.record(&mut ScheduleNameVisitor(&mut system_name));
                system_name
                    .as_deref()
                    .and_then(render_system_label)
                    .map(str::to_owned)
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
                label,
                entered: Instant::now(),
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
        let Some(measurement) = open.measurement.or_else(|| state.active.clone()) else {
            return;
        };
        let start = open.entered.max(measurement.started);
        if exited <= start {
            return;
        }
        let start_s = start.duration_since(measurement.started).as_secs_f64();
        let end_s = exited.duration_since(measurement.started).as_secs_f64();
        state.rows.push(ScheduleTraceRow {
            route: measurement.route,
            repeat: measurement.repeat,
            span: open.label,
            route_elapsed_start_s: start_s,
            route_elapsed_end_s: end_s,
            duration_ms: (exited - start).as_secs_f64() * 1_000.0,
            thread: format!("{:?}", std::thread::current().id()),
        });
    }

    fn on_close(&self, id: tracing::span::Id, _ctx: tracing_subscriber::layer::Context<'_, S>) {
        let mut state = self.recorder.lock();
        state.labels.remove(&id);
        state.open.remove(&id);
    }
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
    fn records_only_the_selected_native_render_system_spans() {
        let recorder = ScheduleTraceRecorder::default();
        recorder.start_repeat("orbit-zoom", 1);
        let subscriber = tracing_subscriber::registry().with(
            ScheduleTraceLayer {
                recorder: recorder.clone(),
            }
            .with_filter(tracing_subscriber::filter::filter_fn(selected_span)),
        );

        tracing::subscriber::with_default(subscriber, || {
            let prepare = tracing::info_span!(
                "system",
                name = "bevy_render::view::window::prepare_windows"
            );
            let _prepare = prepare.enter();
            let pipeline = tracing::info_span!(
                "system",
                name = "bevy_render::render_resource::pipeline_cache::PipelineCache::process_pipeline_queue_system"
            );
            let _pipeline = pipeline.enter();
            let ignored = tracing::info_span!("system", name = "unrelated_render_system");
            let _ignored = ignored.enter();
        });

        let csv = recorder.take_repeat_csv("orbit-zoom", 1);
        assert!(csv.contains("\"render-system:prepare_windows\""));
        assert!(csv.contains("\"render-system:process_pipeline_queue_system\""));
        assert!(!csv.contains("unrelated_render_system"));
    }
}
