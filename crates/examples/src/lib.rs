use {bevy_app::prelude::*, std::time::Duration};

pub fn run_for(
    update_interval: Duration,
    total_duration: Duration,
) -> impl FnOnce(App) -> AppExit + 'static {
    move |mut app| {
        let count = total_duration
            .as_nanos()
            .div_ceil(update_interval.as_nanos());

        for _ in 0..count {
            app.update();
            std::thread::sleep(update_interval);
        }

        AppExit::Success
    }
}
