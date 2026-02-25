use {
    bevy_app::prelude::*,
    std::time::{Duration, Instant},
};

pub fn run_for(
    update_interval: Duration,
    total_duration: Duration,
) -> impl FnOnce(App) -> AppExit + 'static {
    bevy_tasks::IoTaskPool::get_or_init(bevy_tasks::TaskPool::new);
    bevy_tasks::ComputeTaskPool::get_or_init(bevy_tasks::TaskPool::new);
    bevy_tasks::AsyncComputeTaskPool::get_or_init(bevy_tasks::TaskPool::new);

    move |mut app| {
        let end = Instant::now() + total_duration;
        loop {
            let start = Instant::now();
            if start >= end {
                break;
            }
            app.update();
            std::thread::sleep(update_interval.saturating_sub(start.elapsed()));
        }

        AppExit::Success
    }
}
