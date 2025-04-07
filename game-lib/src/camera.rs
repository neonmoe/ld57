use engine::geom::Rect;
use glam::Vec2;

pub struct Camera {
    pub position: Vec2,
    pub size: Vec2,
    pub output_size: Vec2,
}

impl Camera {
    pub fn to_output(&self, rect: Rect) -> Rect {
        let scale = self.output_size / self.size;
        Rect {
            x: (rect.x - self.position.x) * scale.x + self.output_size.x / 2.,
            y: (rect.y - self.position.y) * scale.y + self.output_size.y / 2.,
            w: rect.w * scale.x,
            h: rect.h * scale.y,
        }
    }
}
