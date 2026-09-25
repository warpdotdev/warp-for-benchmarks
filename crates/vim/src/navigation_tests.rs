use string_offset::CharOffset;
use warpui_core::text::point::Point;

use super::*;
use crate::vim::{WordBound, WordType};

#[test]
fn horizontal_boundaries_keep_editor_specific_wrapping() {
    let text = "ab\ncd\n\nef";
    let offset = CharOffset::from(1);
    let max = CharOffset::from(text.chars().count());

    assert_eq!(
        horizontal(
            text,
            offset,
            max,
            4,
            Direction::Forward,
            HorizontalBoundary::Line,
            |_| 2
        ),
        CharOffset::from(2),
    );
    assert_eq!(
        horizontal(
            text,
            CharOffset::from(2),
            max,
            1,
            Direction::Forward,
            HorizontalBoundary::SkipNewlines,
            |_| 0,
        ),
        CharOffset::from(4),
    );
    assert_eq!(
        horizontal(
            text,
            CharOffset::from(2),
            max,
            1,
            Direction::Forward,
            HorizontalBoundary::CodeEditorWrap {
                keep_selection: false,
            },
            |_| 0,
        ),
        CharOffset::from(3),
    );
    assert_eq!(
        horizontal(
            text,
            CharOffset::from(3),
            max,
            1,
            Direction::Backward,
            HorizontalBoundary::CodeEditorWrap {
                keep_selection: true,
            },
            |_| 0,
        ),
        CharOffset::from(2),
    );
    assert_eq!(
        horizontal(
            text,
            CharOffset::from(3),
            max,
            1,
            Direction::Backward,
            HorizontalBoundary::CodeEditorWrap {
                keep_selection: false,
            },
            |_| 0,
        ),
        CharOffset::from(1),
    );
}

#[test]
fn first_nonwhitespace_uses_line_start_for_blank_lines() {
    assert_eq!(
        first_nonwhitespace("  α ", CharOffset::from(1)),
        Some(CharOffset::from(2)),
    );
    assert_eq!(
        first_nonwhitespace(" \t", CharOffset::from(1)),
        Some(CharOffset::zero()),
    );
}

#[test]
fn counted_word_and_paragraph_destinations() {
    let text = "one two\n\nthree";
    let motion = WordMotion::new(Direction::Forward, WordBound::Start, WordType::Default);
    assert_eq!(
        word(text, CharOffset::zero(), 2, &motion),
        Some(CharOffset::from(9)),
    );
    assert_eq!(
        paragraph(
            text,
            CharOffset::zero(),
            CharOffset::from(text.len()),
            CharOffset::zero(),
            1,
            Direction::Forward,
        ),
        CharOffset::from(8),
    );
}

#[test]
fn line_and_jump_destinations_respect_counts_and_row_origin() {
    let origin = Point::new(2, 5);
    assert_eq!(
        line(origin, 7, 3, LineMotion::End, |_| 4, |_| 1),
        Point::new(4, 4)
    );
    assert_eq!(
        line(origin, 7, 3, LineMotion::FirstNonWhitespace, |_| 4, |_| 1),
        Point::new(2, 1)
    );
    assert_eq!(
        vertical(origin, 7, 3, Direction::Forward, 5, |_| 2),
        Point::new(5, 2)
    );
    assert_eq!(jump_to_line(0, 7, 0), 0);
    assert_eq!(jump_to_line(8, 7, 1), 7);
    assert_eq!(
        first_nonwhitespace_step(3, FirstNonWhitespaceMotion::DownMinusOne),
        (Direction::Forward, 2)
    );
}

#[test]
fn bracket_destination_stays_on_the_selected_line() {
    let text = "a(b(c)d)\n(z)";
    assert_eq!(
        matching_bracket(text, CharOffset::zero()),
        Some(CharOffset::from(7)),
    );
    assert_eq!(matching_bracket(text, CharOffset::from(8)), None);
}
