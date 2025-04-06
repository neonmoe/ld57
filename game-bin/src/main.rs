use engine::{Engine, EngineLimits, allocators::LinearAllocator, static_allocator};
use game_lib::Game;
use platform_sdl2::Sdl2Platform;

fn main() {
    #[cfg(feature = "tracing-subscriber")]
    tracing_subscriber::fmt::init();

    let platform = Sdl2Platform::new("game"); // TODO: come up with a name

    static ARENA: &LinearAllocator = static_allocator!(8 * 1024 * 1024);
    let mut engine = Engine::new(
        &platform,
        ARENA,
        EngineLimits {
            frame_arena_size: 2 * 1024 * 1024,
            resource_database_loaded_chunks_count: 32,
            resource_database_buffer_size: 1024 * 1024,
            ..EngineLimits::DEFAULT
        },
    );
    let mut game = Game::new(ARENA, &engine, &platform);

    platform.run_game_loop(&mut engine, |timestamp, platform, engine| {
        game.iterate(engine, platform, timestamp);
    });
}
