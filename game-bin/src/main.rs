use engine::{Engine, EngineLimits, allocators::LinearAllocator, static_allocator};
use game_lib::Game;
use platform_sdl2::Sdl2Platform;

fn main() {
    let platform = Sdl2Platform::new("game"); // TODO: come up with a name
    let mut engine = {
        static ARENA: &LinearAllocator = static_allocator!(64 * 1024 * 1024);
        Engine::new(&platform, ARENA, EngineLimits::DEFAULT)
    };
    let mut game = Game::new();
    platform.run_game_loop(&mut engine, |timestamp, platform, engine| {
        game.iterate(engine, platform, timestamp);
    });
}
