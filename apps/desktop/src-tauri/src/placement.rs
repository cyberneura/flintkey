//! ランチャー窓を置く位置の計算。
//!
//! 窓の API が返す「物理座標」は OS によって意味が違うので、OS を引数で受け取る純粋な関数にしてある。
//!
//! - macOS: 画面全体で一貫しているのはポイント (論理座標)。モニターの位置と大きさは
//!   「ポイント × そのモニターの拡大率」、カーソルの位置は「ポイント × メインモニターの拡大率」で返る。
//!   拡大率の違うモニターが並ぶと物理座標は一貫しないので、ポイントに戻して計算し、論理座標で置く。
//! - Windows / Linux: 画面全体で一貫しているのは物理座標。そのまま計算し、物理座標で置く。

use flintkey_core::Os;
use flintkey_core::settings::Display;

/// 上下の余白 (作業領域の高さ − 窓の高さ) のうち、上に取る割合。
const TOP_RATIO: f64 = 0.22;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    fn scaled(self, factor: f64) -> Rect {
        Rect {
            x: self.x * factor,
            y: self.y * factor,
            width: self.width * factor,
            height: self.height * factor,
        }
    }

    /// 点から矩形までの距離の 2 乗。中にあれば 0。
    fn distance_squared(self, (x, y): (f64, f64)) -> f64 {
        let dx = (self.x - x).max(x - (self.x + self.width)).max(0.0);
        let dy = (self.y - y).max(y - (self.y + self.height)).max(0.0);
        dx * dx + dy * dy
    }
}

/// 窓の API が返したままのモニターの情報。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Monitor {
    pub bounds: Rect,
    /// メニューバーやタスクバーを除いた領域。
    pub work_area: Rect,
    pub scale_factor: f64,
    pub primary: bool,
}

/// 窓を置く位置。OS の API に渡す単位ごとに分けてある。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Position {
    Logical { x: f64, y: f64 },
    Physical { x: f64, y: f64 },
}

/// `window` (論理サイズ) の窓の左上を置く位置。モニターが 1 つも無ければ `None`。
///
/// `cursor` は窓の API が返したままのカーソル位置。取れなかった場合や、どのモニターにも
/// 入っていない場合は、いちばん近いモニター (取れなかった場合はメインモニター) を使う。
pub fn place(
    os: Os,
    monitors: &[Monitor],
    cursor: Option<(f64, f64)>,
    display: Display,
    window: (f64, f64),
) -> Option<Position> {
    let primary = monitors.iter().find(|monitor| monitor.primary).or(monitors.first())?;
    let points = os == Os::Macos;
    // 画面全体で一貫した座標系での、各モニターの領域。
    let global = |monitor: &Monitor, rect: Rect| {
        if points {
            rect.scaled(1.0 / monitor.scale_factor)
        } else {
            rect
        }
    };
    let cursor = cursor.map(|(x, y)| {
        if points {
            (x / primary.scale_factor, y / primary.scale_factor)
        } else {
            (x, y)
        }
    });

    let target = match (display, cursor) {
        (Display::Cursor, Some(cursor)) => monitors
            .iter()
            .min_by(|a, b| {
                let a = global(a, a.bounds).distance_squared(cursor);
                let b = global(b, b.bounds).distance_squared(cursor);
                a.total_cmp(&b)
            })
            .unwrap_or(primary),
        _ => primary,
    };

    let area = global(target, target.work_area);
    let units_per_logical = if points { 1.0 } else { target.scale_factor };
    let (width, height) = (window.0 * units_per_logical, window.1 * units_per_logical);
    // 作業領域より大きい窓は、左上を揃えて切れる側を右下にする。
    let x = area.x + ((area.width - width) / 2.0).max(0.0);
    let y = area.y + ((area.height - height) * TOP_RATIO).max(0.0);

    Some(if points {
        Position::Logical { x, y }
    } else {
        Position::Physical {
            x: x.round(),
            y: y.round(),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const WINDOW: (f64, f64) = (720.0, 480.0);

    fn rect(x: f64, y: f64, width: f64, height: f64) -> Rect {
        Rect { x, y, width, height }
    }

    /// ポイントで書いたモニターを、macOS の窓 API が返す形 (ポイント × 拡大率) にする。
    fn mac_monitor(bounds: Rect, menu_bar: f64, scale_factor: f64, primary: bool) -> Monitor {
        let work_area = rect(bounds.x, bounds.y + menu_bar, bounds.width, bounds.height - menu_bar);
        Monitor {
            bounds: bounds.scaled(scale_factor),
            work_area: work_area.scaled(scale_factor),
            scale_factor,
            primary,
        }
    }

    /// 内蔵 (2 倍、メイン) の右に外付け (1 倍) を置いた構成。カーソルはポイントで受け取る。
    fn mac_retina_and_external(cursor_points: (f64, f64), display: Display) -> Option<Position> {
        let monitors = [
            mac_monitor(rect(0.0, 0.0, 1512.0, 982.0), 32.0, 2.0, true),
            mac_monitor(rect(1512.0, 0.0, 1920.0, 1080.0), 24.0, 1.0, false),
        ];
        // カーソルはメインモニターの拡大率で物理座標にされて返る。
        let cursor = (cursor_points.0 * 2.0, cursor_points.1 * 2.0);
        place(Os::Macos, &monitors, Some(cursor), display, WINDOW)
    }

    #[test]
    fn macos_places_on_the_external_monitor_under_the_cursor() {
        // Arrange / Act
        let position = mac_retina_and_external((2000.0, 500.0), Display::Cursor);

        // Assert
        let x = 1512.0 + (1920.0 - 720.0) / 2.0;
        let y = 24.0 + (1080.0 - 24.0 - 480.0) * TOP_RATIO;
        assert_eq!(position, Some(Position::Logical { x, y }));
    }

    #[test]
    fn macos_places_on_the_builtin_monitor_under_the_cursor() {
        // Arrange / Act
        let position = mac_retina_and_external((700.0, 500.0), Display::Cursor);

        // Assert
        let x = (1512.0 - 720.0) / 2.0;
        let y = 32.0 + (982.0 - 32.0 - 480.0) * TOP_RATIO;
        assert_eq!(position, Some(Position::Logical { x, y }));
    }

    #[test]
    fn macos_cursor_is_not_misjudged_by_the_scale_difference() {
        // Arrange: 内蔵の上の (1000pt, 500pt)。返ってくる値は (2000, 1000) で、物理座標のまま比べると
        // 外付け (1512 から始まる) にも入ってしまう。外付けを先に並べ、先勝ちでは当たらないようにする。
        let monitors = [
            mac_monitor(rect(1512.0, 0.0, 1920.0, 1080.0), 24.0, 1.0, false),
            mac_monitor(rect(0.0, 0.0, 1512.0, 982.0), 32.0, 2.0, true),
        ];

        // Act
        let position = place(Os::Macos, &monitors, Some((2000.0, 1000.0)), Display::Cursor, WINDOW);

        // Assert
        let Some(Position::Logical { x, .. }) = position else {
            panic!("expected a logical position, got {position:?}");
        };
        assert_eq!(x, (1512.0 - 720.0) / 2.0);
    }

    #[test]
    fn macos_external_primary_with_builtin_on_the_left() {
        // Arrange: 外付け (1 倍) がメイン。内蔵 (2 倍) がその左で、座標は負になる。
        let monitors = [
            mac_monitor(rect(0.0, 0.0, 1920.0, 1080.0), 24.0, 1.0, true),
            mac_monitor(rect(-1512.0, 98.0, 1512.0, 982.0), 32.0, 2.0, false),
        ];
        let cursor = (-800.0, 400.0);

        // Act
        let position = place(Os::Macos, &monitors, Some(cursor), Display::Cursor, WINDOW);

        // Assert
        let x = -1512.0 + (1512.0 - 720.0) / 2.0;
        let y = 98.0 + 32.0 + (982.0 - 32.0 - 480.0) * TOP_RATIO;
        assert_eq!(position, Some(Position::Logical { x, y }));
    }

    #[test]
    fn primary_setting_ignores_the_cursor() {
        // Arrange / Act
        let position = mac_retina_and_external((2000.0, 500.0), Display::Primary);

        // Assert
        let Some(Position::Logical { x, .. }) = position else {
            panic!("expected a logical position, got {position:?}");
        };
        assert_eq!(x, (1512.0 - 720.0) / 2.0);
    }

    #[test]
    fn missing_cursor_falls_back_to_the_primary_monitor() {
        // Arrange
        let monitors = [
            mac_monitor(rect(1512.0, 0.0, 1920.0, 1080.0), 24.0, 1.0, false),
            mac_monitor(rect(0.0, 0.0, 1512.0, 982.0), 32.0, 2.0, true),
        ];

        // Act
        let position = place(Os::Macos, &monitors, None, Display::Cursor, WINDOW);

        // Assert
        let Some(Position::Logical { x, .. }) = position else {
            panic!("expected a logical position, got {position:?}");
        };
        assert_eq!(x, (1512.0 - 720.0) / 2.0);
    }

    #[test]
    fn cursor_outside_every_monitor_uses_the_nearest_one() {
        // Arrange: 外付けの下端より下。どのモニターにも入らない。
        let position = mac_retina_and_external((2500.0, 1200.0), Display::Cursor);

        // Assert
        let Some(Position::Logical { x, .. }) = position else {
            panic!("expected a logical position, got {position:?}");
        };
        assert_eq!(x, 1512.0 + (1920.0 - 720.0) / 2.0);
    }

    #[test]
    fn windows_scales_the_window_by_the_target_monitor() {
        // Arrange: 1 倍のメインの右に 2 倍の 4K。Windows は物理座標が画面全体で一貫している。
        let monitors = [
            Monitor {
                bounds: rect(0.0, 0.0, 1920.0, 1080.0),
                work_area: rect(0.0, 0.0, 1920.0, 1040.0),
                scale_factor: 1.0,
                primary: true,
            },
            Monitor {
                bounds: rect(1920.0, 0.0, 3840.0, 2160.0),
                work_area: rect(1920.0, 0.0, 3840.0, 2080.0),
                scale_factor: 2.0,
                primary: false,
            },
        ];

        // Act
        let on_4k = place(Os::Windows, &monitors, Some((3000.0, 500.0)), Display::Cursor, WINDOW);
        let on_main = place(Os::Windows, &monitors, Some((100.0, 500.0)), Display::Cursor, WINDOW);

        // Assert
        assert_eq!(
            on_4k,
            Some(Position::Physical {
                x: 1920.0 + (3840.0 - 1440.0) / 2.0,
                y: ((2080.0 - 960.0) * TOP_RATIO).round(),
            })
        );
        assert_eq!(
            on_main,
            Some(Position::Physical {
                x: (1920.0 - 720.0) / 2.0,
                y: ((1040.0 - 480.0) * TOP_RATIO).round(),
            })
        );
    }

    #[test]
    fn window_larger_than_the_work_area_is_aligned_to_its_origin() {
        // Arrange
        let monitors = [Monitor {
            bounds: rect(0.0, 0.0, 640.0, 400.0),
            work_area: rect(0.0, 30.0, 640.0, 370.0),
            scale_factor: 1.0,
            primary: true,
        }];

        // Act
        let position = place(Os::Windows, &monitors, None, Display::Primary, WINDOW);

        // Assert
        assert_eq!(position, Some(Position::Physical { x: 0.0, y: 30.0 }));
    }

    #[test]
    fn no_monitor_gives_no_position() {
        assert_eq!(place(Os::Macos, &[], None, Display::Cursor, WINDOW), None);
    }
}
