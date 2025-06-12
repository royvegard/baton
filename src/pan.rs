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
        if width % 2 == 0 {
            width -= 1;
        }
        let center = area.left() + width / 2;
        let resolution = width;

        let a = -100.0;
        let b = 100.0;
        let c = 0.0;
        let d = resolution as f64;
        let t = self.balance as f64;

        let value = (c + ((d - c) / (b - a)) * (t - a)) as u16;

        for y in area.top()..area.bottom() {
            for x in area.left()..area.left() + width {
                //buf[(x, 0)].set_symbol("-⡇-  -⢸-");
                if x == area.left() + value {
                    buf[(x, y)].set_symbol("|");
                } else {
                    buf[(x, y)].set_symbol("-");
                }
            }
        }

        if self.balance == 0 {
            buf[(center, area.top())].set_symbol("c");
        } else if self.balance <= -100 {
            buf[(area.left(), area.top())].set_symbol("<");
        } else if self.balance >= 100 {
            buf[(area.left() + width - 1, area.top())].set_symbol(">");
        }

        // Debug strings
        // buf.set_string(
        //     area.left(),
        //     1,
        //     format!("b:{} v:{}", self.balance, value),
        //     Style::new(),
        // );
        // buf.set_string(
        //     area.left(),
        //     2,
        //     format!("c:{} w:{}", center, width),
        //     Style::new(),
        // );
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
