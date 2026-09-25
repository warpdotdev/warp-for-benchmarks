use std::cmp;

use string_offset::CharOffset;
use warpui_core::text::TextBuffer;
use warpui_core::text::point::Point;

use crate::vim::{
    BracketChar, CharacterMotion, Direction, FirstNonWhitespaceMotion, LineMotion, VimMotion,
};
use crate::{
    find_next_paragraph_end, find_previous_paragraph_start, vim_find_char_on_line,
    vim_find_matching_bracket, vim_word_iterator_from_offset,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WrappingMotionBehavior {
    IgnoreNewlines,
    ThroughNewlines,
    StopAtLineBoundary,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LineColumn {
    Start,
    FirstNonWhitespace,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LineNumbering {
    OneBased,
    Direct,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VimNavigationConfig {
    pub wrapping_motion_behavior: WrappingMotionBehavior,
    pub paragraph_start_fallback: usize,
    pub first_line_column: LineColumn,
    pub last_line_column: LineColumn,
    pub numbered_line_column: LineColumn,
    pub line_numbering: LineNumbering,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VimNavigationState {
    pub offset: CharOffset,
    pub goal_column: Option<u32>,
}

pub fn vim_motion_destination<T: TextBuffer + ?Sized>(
    buffer: &T,
    state: VimNavigationState,
    count: u32,
    motion: &VimMotion,
    config: VimNavigationConfig,
) -> VimNavigationState {
    let destination = match motion {
        VimMotion::Character(motion) => match motion {
            CharacterMotion::Left => horizontal_destination(
                buffer,
                state.offset,
                count,
                Direction::Backward,
                WrappingMotionBehavior::StopAtLineBoundary,
            ),
            CharacterMotion::Right => horizontal_destination(
                buffer,
                state.offset,
                count,
                Direction::Forward,
                WrappingMotionBehavior::StopAtLineBoundary,
            ),
            CharacterMotion::WrappingLeft => horizontal_destination(
                buffer,
                state.offset,
                count,
                Direction::Backward,
                config.wrapping_motion_behavior,
            ),
            CharacterMotion::WrappingRight => horizontal_destination(
                buffer,
                state.offset,
                count,
                Direction::Forward,
                config.wrapping_motion_behavior,
            ),
            CharacterMotion::Up => {
                return vertical_destination(buffer, state, count, Direction::Backward);
            }
            CharacterMotion::Down => {
                return vertical_destination(buffer, state, count, Direction::Forward);
            }
        },
        VimMotion::Word(motion) => vim_word_iterator_from_offset(
            state.offset,
            buffer,
            motion.direction,
            motion.bound,
            motion.word_type,
        )
        .ok()
        .and_then(|boundaries| boundaries.take(count as usize).last())
        .unwrap_or(state.offset),
        VimMotion::Line(motion) => match motion {
            LineMotion::Start => line_column_destination(buffer, state.offset, LineColumn::Start),
            LineMotion::FirstNonWhitespace => {
                line_column_destination(buffer, state.offset, LineColumn::FirstNonWhitespace)
            }
            LineMotion::End => {
                let state = vertical_destination(
                    buffer,
                    state,
                    count.saturating_sub(1),
                    Direction::Forward,
                );
                line_end_destination(buffer, state.offset)
            }
        },
        VimMotion::FirstNonWhitespace(motion) => {
            let (count, direction) = match motion {
                FirstNonWhitespaceMotion::Up => (count, Direction::Backward),
                FirstNonWhitespaceMotion::Down => (count, Direction::Forward),
                FirstNonWhitespaceMotion::DownMinusOne => {
                    (count.saturating_sub(1), Direction::Forward)
                }
            };
            let state = vertical_destination(buffer, state, count, direction);
            line_column_destination(buffer, state.offset, LineColumn::FirstNonWhitespace)
        }
        VimMotion::FindChar(motion) => {
            let Some(point) = buffer.to_point(state.offset).ok() else {
                return state;
            };
            let Some(line) = line_text(buffer, state.offset) else {
                return state;
            };
            let Some(column) =
                vim_find_char_on_line(&line, point.column as usize, motion, count, false)
            else {
                return state;
            };
            buffer
                .to_offset(Point::new(point.row, column as u32))
                .unwrap_or(state.offset)
        }
        VimMotion::Paragraph(direction) => {
            let max_offset = max_offset(buffer).unwrap_or(state.offset);
            let fallback = cmp::min(
                max_offset,
                CharOffset::from(config.paragraph_start_fallback),
            );
            let mut offset = state.offset;
            for _ in 0..count {
                offset = match direction {
                    Direction::Forward => {
                        find_next_paragraph_end(buffer, offset).unwrap_or(max_offset)
                    }
                    Direction::Backward => {
                        find_previous_paragraph_start(buffer, offset).unwrap_or(fallback)
                    }
                };
            }
            offset
        }
        VimMotion::JumpToFirstLine => {
            line_on_row_destination(buffer, 0, config.first_line_column).unwrap_or(state.offset)
        }
        VimMotion::JumpToLastLine => {
            let Some(max_point) = max_point(buffer) else {
                return state;
            };
            line_on_row_destination(buffer, max_point.row, config.last_line_column)
                .unwrap_or(state.offset)
        }
        VimMotion::JumpToLine(line_number) => {
            let Some(max_point) = max_point(buffer) else {
                return state;
            };
            let row = match config.line_numbering {
                LineNumbering::OneBased => line_number.saturating_sub(1),
                LineNumbering::Direct => (*line_number).max(1),
            }
            .min(max_point.row);
            line_on_row_destination(buffer, row, config.numbered_line_column)
                .unwrap_or(state.offset)
        }
        VimMotion::JumpToMatchingBracket => {
            matching_bracket_destination(buffer, state.offset).unwrap_or(state.offset)
        }
        VimMotion::JumpToUnmatchedBracket(bracket) => {
            vim_find_matching_bracket(buffer, bracket, state.offset).unwrap_or(state.offset)
        }
    };

    VimNavigationState {
        offset: destination,
        goal_column: None,
    }
}

fn horizontal_destination<T: TextBuffer + ?Sized>(
    buffer: &T,
    offset: CharOffset,
    count: u32,
    direction: Direction,
    behavior: WrappingMotionBehavior,
) -> CharOffset {
    match behavior {
        WrappingMotionBehavior::StopAtLineBoundary => {
            let Some(point) = buffer.to_point(offset).ok() else {
                return offset;
            };
            let distance = match direction {
                Direction::Backward => point.column.min(count),
                Direction::Forward => line_length(buffer, offset)
                    .unwrap_or(point.column)
                    .saturating_sub(point.column)
                    .min(count),
            };
            offset_by(buffer, offset, distance, direction)
        }
        WrappingMotionBehavior::IgnoreNewlines => {
            let chars = match direction {
                Direction::Backward => buffer
                    .chars_rev_at(offset)
                    .ok()
                    .map(|chars| Box::new(chars) as Box<dyn Iterator<Item = char>>),
                Direction::Forward => buffer
                    .chars_at(offset)
                    .ok()
                    .map(|chars| Box::new(chars) as Box<dyn Iterator<Item = char>>),
            };
            let Some(chars) = chars else {
                return offset;
            };
            let mut non_newlines = 0;
            for (index, c) in chars.enumerate() {
                if non_newlines >= count {
                    return offset_by(buffer, offset, index as u32, direction);
                }
                if c != '\n' {
                    non_newlines += 1;
                }
            }
            offset_by(buffer, offset, non_newlines, direction)
        }
        WrappingMotionBehavior::ThroughNewlines => {
            let Some(max_offset) = max_offset(buffer) else {
                return offset;
            };
            let mut destination = offset;
            for _ in 0..count {
                match direction {
                    Direction::Forward => {
                        if destination >= max_offset {
                            break;
                        }
                        let next = cmp::min(max_offset, destination + 1);
                        if char_at(buffer, next) == Some('\n') {
                            let after_next = cmp::min(max_offset, next + 1);
                            destination = if char_at(buffer, after_next) == Some('\n') {
                                next
                            } else {
                                after_next
                            };
                        } else {
                            destination = next;
                        }
                    }
                    Direction::Backward => {
                        if destination <= CharOffset::from(1) {
                            break;
                        }
                        let previous = CharOffset::from(destination.as_usize().saturating_sub(1));
                        if char_at(buffer, previous) == Some('\n') {
                            let before_previous =
                                CharOffset::from(previous.as_usize().saturating_sub(1));
                            destination = if char_at(buffer, before_previous) == Some('\n') {
                                previous
                            } else {
                                before_previous
                            };
                        } else {
                            destination = previous;
                        }
                    }
                }
            }
            destination
        }
    }
}

fn vertical_destination<T: TextBuffer + ?Sized>(
    buffer: &T,
    state: VimNavigationState,
    count: u32,
    direction: Direction,
) -> VimNavigationState {
    let Some(mut point) = buffer.to_point(state.offset).ok() else {
        return state;
    };
    let Some(max_point) = max_point(buffer) else {
        return state;
    };
    let goal_column = state.goal_column.unwrap_or(point.column).max(point.column);
    point.row = match direction {
        Direction::Backward => point.row.saturating_sub(count),
        Direction::Forward => point.row.saturating_add(count).min(max_point.row),
    };
    point.column = goal_column.min(line_length_on_row(buffer, point.row).unwrap_or(point.column));
    VimNavigationState {
        offset: buffer.to_offset(point).unwrap_or(state.offset),
        goal_column: Some(goal_column),
    }
}

fn line_column_destination<T: TextBuffer + ?Sized>(
    buffer: &T,
    offset: CharOffset,
    column: LineColumn,
) -> CharOffset {
    let Some(point) = buffer.to_point(offset).ok() else {
        return offset;
    };
    line_on_row_destination(buffer, point.row, column).unwrap_or(offset)
}

fn line_on_row_destination<T: TextBuffer + ?Sized>(
    buffer: &T,
    row: u32,
    column: LineColumn,
) -> Option<CharOffset> {
    let line_start = buffer.to_offset(Point::new(row, 0)).ok()?;
    match column {
        LineColumn::Start => Some(line_start),
        LineColumn::FirstNonWhitespace => {
            let first_nonwhitespace = buffer
                .chars_at(line_start)
                .ok()?
                .take_while(|c| *c != '\n')
                .position(|c| !c.is_whitespace())
                .unwrap_or(0);
            buffer
                .to_offset(Point::new(row, first_nonwhitespace as u32))
                .ok()
        }
    }
}

fn line_end_destination<T: TextBuffer + ?Sized>(buffer: &T, offset: CharOffset) -> CharOffset {
    let Some(point) = buffer.to_point(offset).ok() else {
        return offset;
    };
    let Some(line_length) = line_length(buffer, offset) else {
        return offset;
    };
    buffer
        .to_offset(Point::new(point.row, line_length))
        .unwrap_or(offset)
}

fn line_length<T: TextBuffer + ?Sized>(buffer: &T, offset: CharOffset) -> Option<u32> {
    let point = buffer.to_point(offset).ok()?;
    line_length_on_row(buffer, point.row)
}

fn line_length_on_row<T: TextBuffer + ?Sized>(buffer: &T, row: u32) -> Option<u32> {
    let line_start = buffer.to_offset(Point::new(row, 0)).ok()?;
    Some(
        buffer
            .chars_at(line_start)
            .ok()?
            .take_while(|c| *c != '\n')
            .count() as u32,
    )
}

fn line_text<T: TextBuffer + ?Sized>(buffer: &T, offset: CharOffset) -> Option<String> {
    let point = buffer.to_point(offset).ok()?;
    let line_start = buffer.to_offset(Point::new(point.row, 0)).ok()?;
    Some(
        buffer
            .chars_at(line_start)
            .ok()?
            .take_while(|c| *c != '\n')
            .collect(),
    )
}

fn matching_bracket_destination<T: TextBuffer + ?Sized>(
    buffer: &T,
    offset: CharOffset,
) -> Option<CharOffset> {
    let chars = buffer.chars_at(offset).ok()?.take_while(|c| *c != '\n');
    let (relative_offset, bracket) = chars
        .enumerate()
        .find_map(|(i, c)| Some((i, BracketChar::try_from(c).ok()?)))?;
    let bracket_offset = offset + relative_offset;
    vim_find_matching_bracket(buffer, &bracket, bracket_offset)
}

fn max_point<T: TextBuffer + ?Sized>(buffer: &T) -> Option<Point> {
    buffer.to_point(max_offset(buffer)?).ok()
}

fn max_offset<T: TextBuffer + ?Sized>(buffer: &T) -> Option<CharOffset> {
    Some(CharOffset::from(
        buffer.chars_at(CharOffset::zero()).ok()?.count(),
    ))
}

fn char_at<T: TextBuffer + ?Sized>(buffer: &T, offset: CharOffset) -> Option<char> {
    buffer.chars_at(offset).ok()?.next()
}

fn offset_by<T: TextBuffer + ?Sized>(
    buffer: &T,
    offset: CharOffset,
    distance: u32,
    direction: Direction,
) -> CharOffset {
    match direction {
        Direction::Backward => {
            CharOffset::from(offset.as_usize().saturating_sub(distance as usize))
        }
        Direction::Forward => cmp::min(
            max_offset(buffer).unwrap_or(offset),
            offset + distance as usize,
        ),
    }
}

#[cfg(test)]
#[path = "navigation_tests.rs"]
mod tests;
