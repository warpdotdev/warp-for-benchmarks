use std::cmp;

use string_offset::CharOffset;
use warpui_core::text::TextBuffer;
use warpui_core::text::point::Point;

use crate::vim::{BracketChar, Direction, FirstNonWhitespaceMotion, LineMotion, WordMotion};
use crate::{
    find_next_paragraph_end, find_previous_paragraph_start, vim_find_matching_bracket,
    vim_word_iterator_from_offset,
};

#[derive(Clone, Copy)]
pub enum HorizontalBoundary {
    Line,
    SkipNewlines,
    CodeEditorWrap { keep_selection: bool },
}
pub fn first_nonwhitespace_step(count: u32, motion: FirstNonWhitespaceMotion) -> (Direction, u32) {
    match motion {
        FirstNonWhitespaceMotion::Up => (Direction::Backward, count),
        FirstNonWhitespaceMotion::Down => (Direction::Forward, count),
        FirstNonWhitespaceMotion::DownMinusOne => (Direction::Forward, count.saturating_sub(1)),
    }
}

pub fn horizontal<T: TextBuffer + ?Sized>(
    buffer: &T,
    origin: CharOffset,
    max: CharOffset,
    count: u32,
    direction: Direction,
    boundary: HorizontalBoundary,
    line_len: impl FnOnce(u32) -> u32,
) -> CharOffset {
    match boundary {
        HorizontalBoundary::Line => {
            let Ok(point) = buffer.to_point(origin) else {
                return origin;
            };
            let line_len = line_len(point.row) as usize;
            let distance = match direction {
                Direction::Backward => cmp::min(point.column as usize, count as usize),
                Direction::Forward => cmp::min(
                    line_len.saturating_sub(point.column as usize),
                    count as usize,
                ),
            };
            match direction {
                Direction::Backward => CharOffset::from(origin.as_usize().saturating_sub(distance)),
                Direction::Forward => cmp::min(max, origin + distance),
            }
        }
        HorizontalBoundary::SkipNewlines => {
            let chars = match direction {
                Direction::Backward => buffer
                    .chars_rev_at(origin)
                    .ok()
                    .map(|iter| Box::new(iter) as Box<dyn Iterator<Item = char> + '_>),
                Direction::Forward => buffer
                    .chars_at(origin)
                    .ok()
                    .map(|iter| Box::new(iter) as Box<dyn Iterator<Item = char> + '_>),
            };
            let Some(chars) = chars else {
                return origin;
            };
            let mut consumed = 0;
            let mut distance = None;
            for (index, c) in chars.enumerate() {
                if consumed >= count {
                    distance = Some(index);
                    break;
                }
                if c != '\n' {
                    consumed += 1;
                }
            }
            let distance = distance.unwrap_or(consumed as usize);
            match direction {
                Direction::Backward => CharOffset::from(origin.as_usize().saturating_sub(distance)),
                Direction::Forward => cmp::min(max, origin + distance),
            }
        }
        HorizontalBoundary::CodeEditorWrap { keep_selection } => {
            let mut offset = origin;
            for _ in 0..count {
                match direction {
                    Direction::Forward => {
                        if offset >= max {
                            break;
                        }
                        let next = cmp::min(max, offset + 1);
                        offset = if buffer
                            .chars_at(next)
                            .ok()
                            .and_then(|mut chars| chars.next())
                            == Some('\n')
                            && !keep_selection
                        {
                            let after_next = cmp::min(max, next + 1);
                            if buffer
                                .chars_at(after_next)
                                .ok()
                                .and_then(|mut chars| chars.next())
                                == Some('\n')
                            {
                                next
                            } else {
                                after_next
                            }
                        } else {
                            next
                        };
                    }
                    Direction::Backward => {
                        if offset <= CharOffset::from(1) {
                            break;
                        }
                        let prev = CharOffset::from(offset.as_usize().saturating_sub(1));
                        offset = if buffer
                            .chars_at(prev)
                            .ok()
                            .and_then(|mut chars| chars.next())
                            == Some('\n')
                            && !keep_selection
                        {
                            let prev2 = CharOffset::from(prev.as_usize().saturating_sub(1));
                            if buffer
                                .chars_at(prev2)
                                .ok()
                                .and_then(|mut chars| chars.next())
                                == Some('\n')
                            {
                                prev
                            } else {
                                prev2
                            }
                        } else {
                            prev
                        };
                    }
                }
            }
            offset
        }
    }
}

pub fn row(origin: u32, max_row: u32, count: u32, direction: Direction) -> u32 {
    match direction {
        Direction::Backward => origin.saturating_sub(count),
        Direction::Forward => cmp::min(max_row, origin.saturating_add(count)),
    }
}

pub fn vertical(
    origin: Point,
    max_row: u32,
    count: u32,
    direction: Direction,
    goal_column: u32,
    target_line_len: impl FnOnce(u32) -> u32,
) -> Point {
    let target_row = row(origin.row, max_row, count, direction);
    Point::new(target_row, goal_column.min(target_line_len(target_row)))
}

pub fn line(
    origin: Point,
    max_row: u32,
    count: u32,
    motion: LineMotion,
    line_len: impl FnOnce(u32) -> u32,
    first_nonwhitespace: impl FnOnce(u32) -> u32,
) -> Point {
    let target_row = if motion == LineMotion::End {
        row(
            origin.row,
            max_row,
            count.saturating_sub(1),
            Direction::Forward,
        )
    } else {
        origin.row
    };
    let column = match motion {
        LineMotion::Start => 0,
        LineMotion::FirstNonWhitespace => first_nonwhitespace(target_row),
        LineMotion::End => line_len(target_row),
    };
    Point::new(target_row, column)
}

pub fn first_nonwhitespace<T: TextBuffer + ?Sized>(
    buffer: &T,
    origin: CharOffset,
) -> Option<CharOffset> {
    let point = buffer.to_point(origin).ok()?;
    let start = buffer.to_offset(Point::new(point.row, 0)).ok()?;
    let line = buffer.chars_at(start).ok()?.take_while(|c| *c != '\n');
    let column = line
        .enumerate()
        .find_map(|(column, c)| (!c.is_whitespace()).then_some(column));
    Some(start + column.unwrap_or(0))
}

pub fn jump_to_line(line_number: u32, max_row: u32, first_row: u32) -> u32 {
    line_number
        .saturating_sub(1)
        .saturating_add(first_row)
        .min(max_row)
}

pub fn word<T: TextBuffer + ?Sized>(
    buffer: &T,
    origin: CharOffset,
    count: u32,
    motion: &WordMotion,
) -> Option<CharOffset> {
    vim_word_iterator_from_offset(
        origin,
        buffer,
        motion.direction,
        motion.bound,
        motion.word_type,
    )
    .ok()
    .map(|boundaries| boundaries.take(count as usize).last().unwrap_or(origin))
}

pub fn paragraph<T: TextBuffer + ?Sized>(
    buffer: &T,
    mut origin: CharOffset,
    max: CharOffset,
    min: CharOffset,
    count: u32,
    direction: Direction,
) -> CharOffset {
    for _ in 0..count {
        origin = match direction {
            Direction::Forward => find_next_paragraph_end(buffer, origin).unwrap_or(max),
            Direction::Backward => find_previous_paragraph_start(buffer, origin).unwrap_or(min),
        };
    }
    origin
}

pub fn matching_bracket<T: TextBuffer + ?Sized>(
    buffer: &T,
    origin: CharOffset,
) -> Option<CharOffset> {
    let mut chars = buffer.chars_at(origin).ok()?.take_while(|c| *c != '\n');
    let current = chars.next()?;
    let (bracket, start) = match BracketChar::try_from(current) {
        Ok(bracket) => (bracket, origin),
        Err(_) => chars
            .enumerate()
            .find_map(|(i, c)| Some((BracketChar::try_from(c).ok()?, origin + i + 1)))?,
    };
    vim_find_matching_bracket(buffer, &bracket, start)
}

#[cfg(test)]
#[path = "navigation_tests.rs"]
mod tests;
