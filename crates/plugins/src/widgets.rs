//! What `page()` returns, read into [`Widget`]s. Everything is bounded:
//! at most [`MAX_WIDGETS`] widgets, containers nest at most
//! [`MAX_WIDGET_DEPTH`] deep, and lists inside widgets are capped.
//!
//! Every widget works in both the normal window and Big Picture.

use rhai::{Array, Dynamic, Map};

pub const MAX_WIDGETS: usize = 500;
pub const MAX_WIDGET_DEPTH: usize = 8;
/// Most points in a chart or sparkline.
const MAX_POINTS: usize = 500;
/// Most options in a select or tabs.
const MAX_OPTIONS: usize = 50;

/// How a chart draws its values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChartKind {
    Line,
    Bar,
}

/// A color a badge or metric can use; the theme picks the real color.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tone {
    #[default]
    Neutral,
    Accent,
    Good,
    Warn,
    Bad,
}

impl Tone {
    fn parse(s: &str) -> Self {
        match s {
            "accent" => Self::Accent,
            "good" | "success" | "green" => Self::Good,
            "warn" | "warning" | "yellow" => Self::Warn,
            "bad" | "danger" | "error" | "red" => Self::Bad,
            _ => Self::Neutral,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Widget {
    Heading(String),
    Text(String),
    Muted(String),
    Button { id: String, text: String },
    Stat { label: String, value: String },
    Separator,
    Row(Vec<Widget>),
    Progress(f32),
    Game(u32),
    Input { id: String, value: String, hint: String },
    Toggle { id: String, text: String, value: bool },
    Link { text: String, page: String, arg: Option<String> },
    Table { columns: Vec<String>, rows: Vec<Vec<String>> },
    Pill(String),
    Image { url: String, width: f32, height: f32 },
    // API 4.
    /// A colored label.
    Badge { text: String, tone: Tone },
    /// A big number with a label, and how it changed.
    Metric { label: String, value: String, change: Option<String>, tone: Tone, hint: String },
    /// A box around other widgets, with an optional title.
    Card { title: String, items: Vec<Widget> },
    /// A titled group of widgets, without a box.
    Section { title: String, items: Vec<Widget> },
    /// Widgets in equal columns, wrapping into rows.
    Grid { columns: usize, items: Vec<Widget> },
    /// Pick one of a few options: (value, label).
    Select { id: String, label: String, options: Vec<(String, String)>, value: String },
    /// A number between `min` and `max`.
    Slider { id: String, label: String, min: f64, max: f64, step: f64, value: f64 },
    /// A row of tabs: (value, label); the plugin shows what goes with the picked one.
    Tabs { id: String, items: Vec<(String, String)>, value: String },
    /// A friend's (or your) picture.
    Avatar { steam_id: u64, size: f32 },
    /// Nothing to show yet: an icon, a title and a line of text.
    Empty { icon: String, title: String, text: String },
    /// Something is loading.
    Loading(String),
    Chart { kind: ChartKind, values: Vec<f64>, labels: Vec<String>, height: f32 },
    /// A small line chart, for a row or a card.
    Sparkline(Vec<f64>),
    /// Text shown when pointing at (or, in Big Picture, focusing) a widget.
    Tooltip { text: String, item: Box<Widget> },
}

impl Widget {
    /// A container of other widgets.
    pub fn children(&self) -> Option<&[Widget]> {
        match self {
            Widget::Row(items) | Widget::Card { items, .. } | Widget::Section { items, .. } | Widget::Grid { items, .. } => Some(items),
            Widget::Tooltip { item, .. } => Some(std::slice::from_ref(item.as_ref())),
            _ => None,
        }
    }
}

pub(crate) fn widgets_bounded(d: Dynamic, depth: usize, budget: &mut usize) -> Vec<Widget> {
    if depth >= MAX_WIDGET_DEPTH || *budget >= MAX_WIDGETS {
        return Vec::new();
    }
    let Some(arr) = d.try_cast::<Array>() else { return Vec::new() };
    let mut out = Vec::new();
    for item in arr {
        if *budget >= MAX_WIDGETS {
            break;
        }
        if let Some(w) = widget_bounded(item, depth, budget) {
            out.push(w);
        }
    }
    out
}

fn numbers(v: Option<&Dynamic>) -> Vec<f64> {
    v.and_then(|v| v.clone().try_cast::<Array>())
        .unwrap_or_default()
        .iter()
        .filter_map(|x| x.as_float().ok().or_else(|| x.as_int().ok().map(|i| i as f64)))
        .filter(|x| x.is_finite())
        .take(MAX_POINTS)
        .collect()
}

/// Options as `["a", "b"]` or `[#{value, label}]`.
fn options(v: Option<&Dynamic>) -> Vec<(String, String)> {
    v.and_then(|v| v.clone().try_cast::<Array>())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|o| match o.clone().try_cast::<Map>() {
            Some(m) => {
                let value = m.get("value").map(|v| v.to_string())?;
                let label = m.get("label").or(m.get("text")).map(|v| v.to_string()).unwrap_or_else(|| value.clone());
                Some((value, label))
            }
            None => {
                let s = o.to_string();
                Some((s.clone(), s))
            }
        })
        .take(MAX_OPTIONS)
        .collect()
}

fn widget_bounded(d: Dynamic, depth: usize, budget: &mut usize) -> Option<Widget> {
    if *budget >= MAX_WIDGETS {
        return None;
    }
    let m = d.try_cast::<Map>()?;
    let s = |k: &str| m.get(k).filter(|v| !v.is_unit()).map(|v| v.to_string()).unwrap_or_default();
    let num = |k: &str| m.get(k).and_then(|v| v.as_float().ok().or_else(|| v.as_int().ok().map(|i| i as f64))).filter(|x| x.is_finite());
    let ty = s("type");
    let container = matches!(ty.as_str(), "row" | "card" | "section" | "grid" | "tooltip");
    if container && depth + 1 >= MAX_WIDGET_DEPTH {
        return None;
    }
    *budget += 1;
    let kids = |budget: &mut usize| m.get("items").cloned().map(|v| widgets_bounded(v, depth + 1, budget)).unwrap_or_default();
    Some(match ty.as_str() {
        "heading" => Widget::Heading(s("text")),
        "text" => Widget::Text(s("text")),
        "muted" => Widget::Muted(s("text")),
        "button" => Widget::Button { id: s("id"), text: s("text") },
        "stat" => Widget::Stat { label: s("label"), value: s("value") },
        "separator" | "divider" => Widget::Separator,
        "progress" => Widget::Progress(num("value").unwrap_or(0.0).clamp(0.0, 1.0) as f32),
        "game" => Widget::Game(m.get("app_id").and_then(|v| v.as_int().ok()).unwrap_or(0) as u32),
        "row" => Widget::Row(kids(budget)),
        "input" => Widget::Input { id: s("id"), value: s("value"), hint: s("hint") },
        "toggle" | "checkbox" => Widget::Toggle { id: s("id"), text: s("text"), value: m.get("value").and_then(|v| v.as_bool().ok()).unwrap_or(false) },
        "link" => Widget::Link { text: s("text"), page: s("page"), arg: m.get("arg").filter(|v| !v.is_unit()).map(|v| v.to_string()) },
        "pill" => Widget::Pill(s("text")),
        "table" => {
            let list = |v: Option<&Dynamic>| -> Vec<Dynamic> { v.and_then(|v| v.clone().try_cast::<Array>()).unwrap_or_default() };
            Widget::Table {
                columns: list(m.get("columns")).iter().map(|c| c.to_string()).take(12).collect(),
                rows: list(m.get("rows")).into_iter().take(500).map(|r| r.try_cast::<Array>().unwrap_or_default().iter().map(|c| c.to_string()).take(12).collect()).collect(),
            }
        }
        "image" => Widget::Image {
            url: s("url"),
            width: m.get("width").and_then(|v| v.as_int().ok()).unwrap_or(240).clamp(16, 1200) as f32,
            height: m.get("height").and_then(|v| v.as_int().ok()).unwrap_or(135).clamp(16, 1200) as f32,
        },
        "badge" => Widget::Badge { text: s("text"), tone: Tone::parse(&s("tone")) },
        "metric" => Widget::Metric {
            label: s("label"),
            value: s("value"),
            change: Some(s("change")).filter(|c| !c.is_empty()),
            // Without a tone, a change starting with "-" is bad and one with "+" good.
            tone: match s("tone").as_str() {
                "" if s("change").starts_with('-') => Tone::Bad,
                "" if s("change").starts_with('+') => Tone::Good,
                t => Tone::parse(t),
            },
            hint: s("hint"),
        },
        "card" => Widget::Card { title: s("title"), items: kids(budget) },
        "section" => Widget::Section { title: s("title"), items: kids(budget) },
        "grid" => Widget::Grid { columns: num("columns").unwrap_or(2.0).clamp(1.0, 6.0) as usize, items: kids(budget) },
        "select" => {
            let options = options(m.get("options"));
            let value = Some(s("value")).filter(|v| options.iter().any(|(o, _)| o == v)).or_else(|| options.first().map(|o| o.0.clone())).unwrap_or_default();
            Widget::Select { id: s("id"), label: s("label"), options, value }
        }
        "slider" => {
            let min = num("min").unwrap_or(0.0);
            let max = num("max").unwrap_or(100.0).max(min);
            let step = num("step").unwrap_or((max - min) / 100.0).max((max - min) / 1000.0).max(f64::EPSILON);
            Widget::Slider { id: s("id"), label: s("label"), min, max, step, value: num("value").unwrap_or(min).clamp(min, max) }
        }
        "tabs" => {
            let items = options(m.get("items").or(m.get("options")));
            let value = Some(s("value")).filter(|v| items.iter().any(|(o, _)| o == v)).or_else(|| items.first().map(|o| o.0.clone())).unwrap_or_default();
            Widget::Tabs { id: s("id"), items, value }
        }
        "avatar" => Widget::Avatar {
            steam_id: s("steam_id").parse().unwrap_or(0),
            size: m.get("size").and_then(|v| v.as_int().ok()).unwrap_or(48).clamp(16, 256) as f32,
        },
        "empty_state" | "empty" => Widget::Empty { icon: s("icon"), title: s("title"), text: s("text") },
        "loading" => Widget::Loading(s("text")),
        "chart" => Widget::Chart {
            kind: if s("kind") == "bar" { ChartKind::Bar } else { ChartKind::Line },
            values: numbers(m.get("values")),
            labels: m.get("labels").and_then(|v| v.clone().try_cast::<Array>()).unwrap_or_default().iter().map(|l| l.to_string()).take(MAX_POINTS).collect(),
            height: m.get("height").and_then(|v| v.as_int().ok()).unwrap_or(160).clamp(60, 600) as f32,
        },
        "sparkline" => Widget::Sparkline(numbers(m.get("values"))),
        "tooltip" => {
            let inner = m.get("item").cloned().and_then(|v| widget_bounded(v, depth + 1, budget));
            match inner {
                Some(item) => Widget::Tooltip { text: s("text"), item: Box::new(item) },
                None => {
                    *budget = budget.saturating_sub(1);
                    return None;
                }
            }
        }
        _ => {
            *budget = budget.saturating_sub(1);
            return None;
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::plugin;
    use std::collections::BTreeSet;

    fn page(script: &str) -> Vec<Widget> {
        let mut p = plugin("id = \"wid\"\nname = \"W\"\n[page]\ntitle = \"W\"\n", &format!("fn page() {{ {script} }}"));
        p.start(&BTreeSet::new(), Default::default());
        assert!(p.error.is_none(), "{:?}", p.error);
        p.page()
    }

    #[test]
    fn new_widgets_are_read() {
        let w = page(
            r#"[
                #{type: "badge", text: "Sale", tone: "good"},
                #{type: "metric", label: "Value", value: "$12", change: "-3%"},
                #{type: "card", title: "Box", items: [#{type: "text", text: "in"}]},
                #{type: "section", title: "Part", items: []},
                #{type: "grid", columns: 9, items: [#{type: "pill", text: "a"}]},
                #{type: "select", id: "s", label: "Sort", options: ["name", #{value: "time", label: "Playtime"}], value: "time"},
                #{type: "slider", id: "v", label: "Volume", min: 0, max: 10, value: 42},
                #{type: "tabs", id: "t", items: ["one", "two"], value: "nope"},
                #{type: "avatar", steam_id: "76561197960265728", size: 9999},
                #{type: "empty_state", icon: "trophy", title: "Nothing", text: "Yet"},
                #{type: "loading", text: "Wait"},
                #{type: "chart", kind: "bar", values: [1, 2.5, "x"], labels: ["a", "b"]},
                #{type: "sparkline", values: [3, 1, 2]},
                #{type: "tooltip", text: "Hi", item: #{type: "button", id: "b", text: "B"}},
                #{type: "checkbox", id: "c", text: "C", value: true},
                #{type: "divider"},
            ]"#,
        );
        assert_eq!(w.len(), 16, "{w:?}");
        assert_eq!(w[0], Widget::Badge { text: "Sale".into(), tone: Tone::Good });
        assert!(matches!(&w[1], Widget::Metric { tone: Tone::Bad, change: Some(c), .. } if c == "-3%"));
        assert!(matches!(&w[2], Widget::Card { title, items } if title == "Box" && items.len() == 1));
        assert!(matches!(&w[4], Widget::Grid { columns: 6, .. }), "columns are capped at 6");
        assert!(matches!(&w[5], Widget::Select { options, value, .. } if options[1] == ("time".to_string(), "Playtime".to_string()) && value == "time"));
        assert!(matches!(&w[6], Widget::Slider { value, max, .. } if *value == 10.0 && *max == 10.0), "the value stays in range");
        assert!(matches!(&w[7], Widget::Tabs { value, .. } if value == "one"), "an unknown value picks the first tab");
        assert!(matches!(&w[8], Widget::Avatar { steam_id: 76561197960265728, size } if *size == 256.0));
        assert!(matches!(&w[11], Widget::Chart { kind: ChartKind::Bar, values, .. } if values == &vec![1.0, 2.5]));
        assert!(matches!(&w[13], Widget::Tooltip { item, .. } if matches!(item.as_ref(), Widget::Button { .. })));
        assert!(matches!(&w[14], Widget::Toggle { value: true, .. }));
        assert_eq!(w[15], Widget::Separator);
    }

    #[test]
    fn containers_count_toward_the_limits() {
        let deep = page("let w = #{type: \"text\", text: \"leaf\"}; for i in 0..12 { w = #{type: \"card\", items: [w]}; } [w]");
        fn depth(ws: &[Widget]) -> usize {
            ws.iter().map(|w| w.children().map(|c| 1 + depth(c)).unwrap_or(0)).max().unwrap_or(0)
        }
        assert!(depth(&deep) < MAX_WIDGET_DEPTH);
        let many = page("let a = []; for i in 0..800 { a.push(#{type: \"badge\", text: \"x\"}); } [#{type: \"grid\", items: a}]");
        fn count(ws: &[Widget]) -> usize {
            ws.iter().map(|w| 1 + w.children().map(count).unwrap_or(0)).sum()
        }
        assert_eq!(count(&many), MAX_WIDGETS);
        let points = page("let v = []; for i in 0..900 { v.push(i); } [#{type: \"sparkline\", values: v}]");
        assert!(matches!(&points[0], Widget::Sparkline(v) if v.len() == 500));
    }
}
