use std::time::SystemTime;

use engine::{Engine, EngineLimits, allocators::LinearAllocator, static_allocator};
use game_lib::Game;
use platform_sdl2::Sdl2Platform;

fn main() {
    #[cfg(feature = "tracing-subscriber")]
    tracing_subscriber::fmt::init();

    let platform = Sdl2Platform::new("game"); // TODO: come up with a name

    static ARENA: &LinearAllocator = static_allocator!(16 * 1024 * 1024);
    let mut engine = Engine::new(
        &platform,
        ARENA,
        EngineLimits {
            frame_arena_size: 4 * 1024 * 1024,
            resource_database_loaded_chunks_count: 64,
            resource_database_buffer_size: 1024 * 1024,
            ..EngineLimits::DEFAULT
        },
    );
    let seed = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|t| t.as_secs())
        .unwrap_or(0);
    let mut game = Game::new(ARENA, &engine, &platform, seed);

    platform.run_game_loop(&mut engine, |timestamp, platform, engine| {
        game.iterate(engine, platform, timestamp);
    });
}
