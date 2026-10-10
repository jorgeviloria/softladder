//! The data table shared by the tag, watch and problems documents.
//!
//! `docs/UX.md` §6, §7 and §9 ask for one table: a fixed header, zebra rows, a
//! filter, column sorting and a per-row menu. The cells themselves stay with
//! the document, because a tag name is an editor and a live value is not.

use std::cmp::Ordering;

use egui::{Color32, CornerRadius, Layout, RichText, ScrollArea, Shape, Stroke, Ui, Vec2};

use crate::design::{Tokens, TypeScale, SPACE_1, SPACE_2};

/// How wide one column is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Width {
    /// An exact width, in points.
    Fixed(f32),
    /// A share of whatever width remains after the fixed columns.
    ///
    /// `min` is the width the column keeps when the window is narrow; the extra
    /// space is split by `weight`.
    Flex {
        /// Share of the spare width. Zero or negative is treated as zero.
        weight: f32,
        /// Width the column keeps before any spare width is shared.
        min: f32,
    },
}

/// One column of a [`Spec`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Column {
    /// The header text.
    pub title: &'static str,
    /// How wide the column is.
    pub width: Width,
}

/// What [`show`] draws, apart from the closures that fill the cells.
#[derive(Debug, Clone, Copy)]
pub struct Spec<'a> {
    /// Stable id salt, so two tables on one screen do not share a filter.
    pub id: &'static str,
    /// Columns, left to right.
    pub columns: &'a [Column],
    /// How many rows the caller has, before filtering.
    pub rows: usize,
}

/// The filter and sort of one table, kept in egui's memory.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Memory {
    /// The text in the filter field.
    filter: String,
    /// The column the rows are ordered by, when the user has chosen one.
    sort_column: Option<usize>,
    /// `true` when that column reads low to high.
    ascending: bool,
}

impl Default for Memory {
    fn default() -> Self {
        Self {
            filter: String::new(),
            sort_column: None,
            ascending: true,
        }
    }
}

/// The original indexes of the rows that pass `filter`, in display order.
///
/// A sort key that parses as an integer is ordered numerically, so `2` stays
/// before `10`. Everything else is ordered as text, ignoring case.
pub fn visible_rows(
    rows: usize,
    filter: &str,
    mut haystack: impl FnMut(usize) -> String,
    sort: Option<(usize, bool)>,
    mut sort_key: impl FnMut(usize, usize) -> String,
) -> Vec<usize> {
    let needle = filter.trim().to_ascii_lowercase();
    let mut indexes: Vec<usize> = (0..rows)
        .filter(|index| {
            needle.is_empty() || haystack(*index).to_ascii_lowercase().contains(&needle)
        })
        .collect();
    if let Some((column, ascending)) = sort {
        indexes.sort_by(|left, right| {
            let order = cmp_key(&sort_key(*left, column), &sort_key(*right, column));
            if ascending {
                order
            } else {
                order.reverse()
            }
        });
    }
    indexes
}

/// Compares two sort keys, numerically when both are integers.
fn cmp_key(left: &str, right: &str) -> Ordering {
    let left = left.trim();
    let right = right.trim();
    if let (Ok(left), Ok(right)) = (left.parse::<i64>(), right.parse::<i64>()) {
        return left.cmp(&right);
    }
    left.to_ascii_lowercase().cmp(&right.to_ascii_lowercase())
}

/// Width of each column for a table that has `available` points to spend.
///
/// Fixed columns keep their width and flexible columns share the rest, down to
/// their minimum. When the window is narrower than that, every column shrinks
/// by the same factor so the header and the rows stay aligned.
pub fn column_widths(available: f32, columns: &[Column]) -> Vec<f32> {
    let gap = SPACE_2 * columns.len().saturating_sub(1) as f32;
    let usable = (available - gap).max(1.0);
    let mut widths: Vec<f32> = columns
        .iter()
        .map(|column| match column.width {
            Width::Fixed(width) => width,
            Width::Flex { min, .. } => min,
        })
        .collect();
    let sum: f32 = widths.iter().sum();
    if sum <= 0.0 {
        return widths;
    }
    if sum > usable {
        let scale = usable / sum;
        for width in &mut widths {
            *width *= scale;
        }
        return widths;
    }
    let weight: f32 = columns
        .iter()
        .map(|column| match column.width {
            Width::Flex { weight, .. } => weight.max(0.0),
            Width::Fixed(_) => 0.0,
        })
        .sum();
    if weight <= 0.0 {
        return widths;
    }
    let extra = usable - sum;
    for (width, column) in widths.iter_mut().zip(columns) {
        if let Width::Flex { weight: share, .. } = column.width {
            *width += extra * (share.max(0.0) / weight);
        }
    }
    widths
}

/// Draws the filter, the header and the rows.
///
/// `haystack` is the text the filter searches. `sort_key` is the text of one
/// cell for ordering. `tint` paints a row (a forced watch row, a selection)
/// in place of the zebra. `cell` draws one cell; `menu` fills the row menu.
#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut Ui,
    tokens: &Tokens,
    spec: Spec<'_>,
    haystack: impl Fn(usize) -> String,
    sort_key: impl Fn(usize, usize) -> String,
    tint: impl Fn(usize) -> Option<Color32>,
    mut cell: impl FnMut(&mut Ui, usize, usize),
    mut menu: impl FnMut(&mut Ui, usize),
) {
    let id = ui.make_persistent_id(spec.id);
    let mut memory = ui.data(|data| data.get_temp::<Memory>(id).unwrap_or_default());

    ui.add(
        egui::TextEdit::singleline(&mut memory.filter)
            .hint_text("Filter")
            .desired_width(f32::INFINITY),
    );
    ui.add_space(SPACE_1);

    // The scrollbar sits inside the scroll area, so the header gives it the
    // same room or the last column drifts when the bar appears.
    let available = (ui.available_width() - 8.0).max(1.0);
    let widths = column_widths(available, spec.columns);
    header(ui, tokens, spec.columns, &widths, &mut memory);

    let order = visible_rows(
        spec.rows,
        &memory.filter,
        &haystack,
        memory.sort_column.map(|column| (column, memory.ascending)),
        &sort_key,
    );
    let matched = order.len();
    let max_height = ui.available_height().max(80.0);
    ScrollArea::vertical()
        .auto_shrink([false, true])
        .max_height(max_height)
        .show(ui, |ui| {
            if spec.rows > 0 && matched == 0 {
                ui.add_space(SPACE_2);
                ui.label(
                    RichText::new("No rows match the filter")
                        .size(TypeScale::CAPTION)
                        .color(tokens.text_dim),
                );
            }
            for (stripe, index) in order.into_iter().enumerate() {
                draw_row(
                    ui,
                    tokens,
                    &widths,
                    stripe % 2 == 1,
                    tint(index),
                    |ui, column| cell(ui, index, column),
                    |ui| menu(ui, index),
                );
            }
        });

    ui.data_mut(|data| data.insert_temp(id, memory));
}

/// The header row. Clicking a title orders the table by that column.
fn header(ui: &mut Ui, tokens: &Tokens, columns: &[Column], widths: &[f32], memory: &mut Memory) {
    let response = ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = SPACE_2;
        for (index, column) in columns.iter().enumerate() {
            let width = widths.get(index).copied().unwrap_or(0.0);
            let mark = match memory.sort_column {
                Some(sorted) if sorted == index && memory.ascending => " ▲",
                Some(sorted) if sorted == index => " ▼",
                _ => "",
            };
            let title = format!("{}{mark}", column.title);
            ui.allocate_ui_with_layout(
                Vec2::new(width, 18.0),
                Layout::left_to_right(egui::Align::Center),
                |ui| {
                    // The slot must keep the column width. A button only as wide
                    // as its title would pull the next header left of its column.
                    ui.set_min_width(width);
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new(title)
                                    .size(TypeScale::CAPTION)
                                    .color(tokens.text_dim)
                                    .strong(),
                            )
                            .frame(false),
                        )
                        .on_hover_text(format!("Sort by {}", column.title))
                        .clicked()
                    {
                        if memory.sort_column == Some(index) {
                            memory.ascending = !memory.ascending;
                        } else {
                            memory.sort_column = Some(index);
                            memory.ascending = true;
                        }
                    }
                },
            );
        }
        // The row menu has no column of its own.
        ui.add_space(28.0);
    });
    ui.painter().hline(
        response.response.rect.x_range(),
        response.response.rect.bottom(),
        Stroke::new(1.0_f32, tokens.border),
    );
    ui.add_space(SPACE_1);
}

/// One body row: a zebra or tinted background, the cells, and the row menu.
fn draw_row(
    ui: &mut Ui,
    tokens: &Tokens,
    widths: &[f32],
    stripe: bool,
    tint: Option<Color32>,
    mut cell: impl FnMut(&mut Ui, usize),
    mut menu: impl FnMut(&mut Ui),
) {
    let background = ui.painter().add(Shape::Noop);
    let inner = ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = SPACE_2;
        for (column, width) in widths.iter().copied().enumerate() {
            ui.scope(|ui| {
                ui.set_min_width(width);
                ui.set_max_width(width);
                cell(ui, column);
            });
        }
        let button = ui
            .add_sized(
                Vec2::new(24.0, 18.0),
                egui::Button::new(
                    RichText::new("⋯")
                        .size(TypeScale::BODY)
                        .color(tokens.text_dim),
                )
                .frame(false),
            )
            .on_hover_text("Row actions");
        button.context_menu(|ui| menu(ui));
    });
    let rect = inner.response.rect;
    let hovered = ui.rect_contains_pointer(rect);
    let fill = if hovered {
        tokens.accent_soft
    } else if let Some(tint) = tint {
        tint
    } else if stripe {
        tokens.surface
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().set(
        background,
        Shape::rect_filled(rect, CornerRadius::ZERO, fill),
    );
    ui.painter()
        .hline(rect.x_range(), rect.bottom(), tokens.hairline());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_filter_is_case_insensitive_and_keeps_the_original_index() {
        let names = ["Start", "STOP", "amber"];
        let shown = visible_rows(
            names.len(),
            "st",
            |index| names[index].to_owned(),
            None,
            |_, _| String::new(),
        );
        assert_eq!(shown, vec![0, 1]);
    }

    #[test]
    fn an_empty_filter_shows_every_row_unsorted() {
        let shown = visible_rows(3, "   ", |_| String::new(), None, |_, _| String::new());
        assert_eq!(shown, vec![0, 1, 2]);
    }

    #[test]
    fn text_sorts_ignoring_case_and_numbers_sort_as_numbers() {
        let names = ["start", "Amber", "stop"];
        let ascending = visible_rows(
            names.len(),
            "",
            |_| String::new(),
            Some((0, true)),
            |index, _| names[index].to_owned(),
        );
        assert_eq!(ascending, vec![1, 0, 2], "Amber, start, stop");

        let counts = [10, 2, 1];
        let numeric = visible_rows(
            counts.len(),
            "",
            |_| String::new(),
            Some((0, false)),
            |index, _| counts[index].to_string(),
        );
        assert_eq!(numeric, vec![0, 1, 2], "10, 2, 1 when descending");
    }

    #[test]
    fn flexible_columns_share_the_spare_width_and_shrink_together() {
        let columns = [
            Column {
                title: "Name",
                width: Width::Flex {
                    weight: 2.0,
                    min: 40.0,
                },
            },
            Column {
                title: "Type",
                width: Width::Fixed(20.0),
            },
            Column {
                title: "Comment",
                width: Width::Flex {
                    weight: 1.0,
                    min: 20.0,
                },
            },
        ];
        // Gap is 8 between columns: usable = 100 - 16 = 84.
        // Minimums are 40 + 20 + 20 = 80, so 4 points of spare go 2:1.
        let widths = column_widths(100.0, &columns);
        assert!(
            (widths[1] - 20.0).abs() < 0.01,
            "the fixed column stays put"
        );
        assert!((widths[0] - 42.666).abs() < 0.05);
        assert!((widths[2] - 21.333).abs() < 0.05);

        let narrow = column_widths(40.0, &columns);
        let sum: f32 = narrow.iter().sum();
        let gaps = SPACE_2 * 2.0;
        assert!(
            sum <= 40.0 - gaps + 0.05,
            "a narrow table does not overflow"
        );
        assert!(narrow.iter().all(|width| *width > 0.0));
    }
}
