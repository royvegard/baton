use ratatui::{buffer::Buffer, layout::Rect, widgets::Widget};

#[derive(Debug, Default, Clone, Eq, PartialEq, Hash)]
pub struct Pan {
    pub balance: i64,
}

impl Pan {
    pub fn balance(&mut self, value: i64) {
        self.balance = value;
    }

    fn render_pan(&self, area: Rect, buf: &mut Buffer) {
        if area.is_empty() {
            return;
        }

        let mut width = area.width;
        if width.is_multiple_of(2) {
            width -= 1;
        }
        let center = area.left() + width / 2;

        // Use sub-character resolution (6 horizontal positions per character)
        let resolution = (width as f64) * 6.0;

        let a = -100.0;
        let b = 100.0;
        let c = 0.0;
        let d = resolution;
        let t = self.balance as f64;

        let pixel_position = c + ((d - c) / (b - a)) * (t - a);
        let char_position = (pixel_position / 6.0) as u16;
        let sub_position = (pixel_position % 6.0) as usize;

        // Thin vertical bars at different horizontal positions
        let bars = [
            "🭰", // Leftmost
            "🭱", // Left-center
            "🭲", // Center-left
            "🭳", // Center-right
            "🭴", // Right-center
            "🭵", // Rightmost
        ];

        for y in area.top()..area.bottom() {
            for x in area.left()..area.left() + width {
                let relative_x = x - area.left();

                if relative_x == char_position {
                    // Position indicator with sub-character precision
                    buf[(x, y)].set_symbol(bars[sub_position]);
                } else {
                    // Background line
                    buf[(x, y)].set_symbol("─");
                }
            }
        }

        // Special indicators for extremes and center
        if self.balance == 0 {
            buf[(center, area.top())].set_symbol("█");
        } else if self.balance <= -100 {
            buf[(area.left(), area.top())].set_symbol("◀");
        } else if self.balance >= 100 {
            buf[(area.left() + width - 1, area.top())].set_symbol("▶");
        }
    }
}

impl Widget for Pan {
    fn render(self, area: Rect, buf: &mut Buffer)
    where
        Self: Sized,
    {
        self.render_pan(area, buf);
    }
}
