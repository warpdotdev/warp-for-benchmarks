use anyhow::{Result, anyhow};

use super::*;
use crate::vim::FindCharDestination;

/// Multi-line text whose points are rows and columns, like an editor buffer.
struct Lines(&'static str);

impl TextBuffer for Lines {
    type Chars<'a> = std::str::Chars<'a>;
    type CharsReverse<'a> = std::iter::Rev<std::str::Chars<'a>>;

    fn chars_at(&self, offset: CharOffset) -> Result<Self::Chars<'_>> {
        self.0.chars_at(offset)
    }

    fn chars_rev_at(&self, offset: CharOffset) -> Result<Self::CharsReverse<'_>> {
        self.0.chars_rev_at(offset)
    }

    fn to_point(&self, offset: CharOffset) -> Result<Point> {
        if offset > self.vim_max_offset() {
            return Err(anyhow!("offset {offset} out of bounds"));
        }
        let before: String = self.0.chars().take(offset.as_usize()).collect();
        let row = before.matches('\n').count() as u32;
        let column = before.chars().rev().take_while(|c| *c != '\n').count() as u32;
        Ok(Point::new(row, column))
    }

    fn to_offset(&self, point: Point) -> Result<CharOffset> {
        let preceding_rows: usize = self
            .0
            .split('\n')
            .take(point.row as usize)
            .map(|line| line.chars().count() + 1)
            .sum();
        Ok(CharOffset::from(preceding_rows + point.column as usize))
    }
}

impl VimTextBuffer for Lines {
    fn vim_line_len(&self, row: u32) -> u32 {
        self.0
            .split('\n')
            .nth(row as usize)
            .map_or(0, |line| line.chars().count() as u32)
    }

    fn vim_min_offset(&self) -> CharOffset {
        CharOffset::zero()
    }

    fn vim_max_offset(&self) -> CharOffset {
        CharOffset::from(self.0.chars().count())
    }
}

fn find(direction: Direction, c: char) -> FindCharMotion {
    FindCharMotion {
        direction,
        destination: FindCharDestination::AtChar,
        is_repetition: false,
        c,
    }
}

#[test]
fn test_line_bounded_destination_stops_at_line_boundaries() {
    let buffer = Lines("ab\ncd\nef");
    let head = CharOffset::from(4);

    assert_eq!(
        vim_line_bounded_destination(&buffer, head, 5, Direction::Backward),
        Some(CharOffset::from(3))
    );
    assert_eq!(
        vim_line_bounded_destination(&buffer, head, 5, Direction::Forward),
        Some(CharOffset::from(5))
    );
}

#[test]
fn test_find_char_destination_only_searches_the_cursor_line() {
    let buffer = Lines("xa\nbxa\nx");
    let head = CharOffset::from(3);

    assert_eq!(
        vim_find_char_destination(&buffer, head, &find(Direction::Forward, 'a'), 1, false),
        Some(CharOffset::from(5))
    );
    assert_eq!(
        vim_find_char_destination(&buffer, head, &find(Direction::Forward, 'x'), 2, false),
        None
    );
    assert_eq!(
        vim_find_char_destination(
            &buffer,
            CharOffset::from(5),
            &find(Direction::Backward, 'b'),
            1,
            false
        ),
        Some(CharOffset::from(3))
    );
}

#[test]
fn test_paragraph_destination_falls_back_to_buffer_bounds() {
    let buffer = Lines("a\n\nb\n\nc");
    let head = CharOffset::from(3);

    assert_eq!(
        vim_paragraph_destination(&buffer, head, Direction::Forward, 1),
        CharOffset::from(5)
    );
    assert_eq!(
        vim_paragraph_destination(&buffer, head, Direction::Forward, 3),
        buffer.vim_max_offset()
    );
    assert_eq!(
        vim_paragraph_destination(&buffer, head, Direction::Backward, 3),
        buffer.vim_min_offset()
    );
}

#[test]
fn test_matching_bracket_destination_starts_from_a_bracket_on_the_cursor_line() {
    assert_eq!(
        vim_matching_bracket_destination(&Lines("x (a)\n(b)"), CharOffset::zero()),
        Some(CharOffset::from(4))
    );
    assert_eq!(
        vim_matching_bracket_destination(&Lines("x\n(b)"), CharOffset::zero()),
        None
    );
}

#[test]
fn test_line_first_nonwhitespace_stays_at_start_of_blank_line() {
    let buffer = Lines("a\n  \n  b");

    assert_eq!(
        vim_line_first_nonwhitespace(&buffer, CharOffset::from(3)),
        Some(CharOffset::from(2))
    );
    assert_eq!(
        vim_line_first_nonwhitespace(&buffer, CharOffset::from(5)),
        Some(CharOffset::from(7))
    );
}
