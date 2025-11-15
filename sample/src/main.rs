use bevy::prelude::*;
// Entry point of the application
fn main() {
    App::new()
        .add_systems(Update, hello_world_system) // Add a system to the update schedule
        .run(); // Run the application
}

// A system that prints the current time
fn hello_world_system(mut app_exit_events: MessageWriter<bevy::app::AppExit>) {
    println!("hello world");
    app_exit_events.write(bevy::app::AppExit::Success);
}
