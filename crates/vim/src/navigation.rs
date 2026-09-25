use string_offset::CharOffset;
use warpui_core::text::TextBuffer;
use warpui_core::text::point::Point;
use warpui_core::{Entity, ModelContext};

use crate::vim::{
    BracketChar, CharacterMotion, Direction, FindCharMotion, FirstNonWhitespaceMotion, LineMotion,
    VimMotion, WordMotion,
};
use crate::{
    find_next_paragraph_end, find_previous_paragraph_start, vim_find_char_on_line,
    vim_find_matching_bracket, vim_word_iterator_from_offset,
};

/// A [`TextBuffer`] with the line and bounds information that vim motion destinations are
/// resolved against.
pub trait VimTextBuffer: TextBuffer {
    /// The number of characters on `row`, excluding its line break.
    fn vim_line_len(&self, row: u32) -> u32;

    /// The lowest offset a cursor can occupy.
    fn vim_min_offset(&self) -> CharOffset;

    /// The highest offset a cursor can occupy.
    fn vim_max_offset(&self) -> CharOffset;
}

/// The cursor movement an editor model provides so that [`vim_navigate`] can apply vim motions to
/// it. Only movement that depends on editor-specific behavior (goal columns, crossing line breaks,
/// line numbering) is left to the editor.
pub trait VimCursorModel: Entity + Sized {
    type Buffer: VimTextBuffer + ?Sized;

    /// Moves every cursor to the offset `destination` resolves from its head, or leaves it in place
    /// when that is `None`.
    fn vim_move_cursors<F>(&mut self, destination: F, ctx: &mut ModelContext<Self>)
    where
        F: Fn(&Self::Buffer, CharOffset) -> Option<CharOffset>;

    /// Moves every cursor `count` lines, towards its goal column.
    fn vim_move_vertically(
        &mut self,
        count: u32,
        direction: Direction,
        ctx: &mut ModelContext<Self>,
    );

    /// Moves every cursor `count` characters, continuing past line breaks.
    fn vim_move_across_lines(
        &mut self,
        count: u32,
        direction: Direction,
        ctx: &mut ModelContext<Self>,
    );

    /// `gg` without a count.
    fn vim_jump_to_first_line(&mut self, ctx: &mut ModelContext<Self>);

    /// `G` without a count.
    fn vim_jump_to_last_line(&mut self, ctx: &mut ModelContext<Self>);

    /// `gg` or `G` preceded by `line_number`.
    fn vim_jump_to_line(&mut self, line_number: u32, ctx: &mut ModelContext<Self>);
}

/// Moves the cursors of `model` according to a vim navigation motion repeated `count` times.
pub fn vim_navigate<M: VimCursorModel>(
    model: &mut M,
    count: u32,
    motion: &VimMotion,
    ctx: &mut ModelContext<M>,
) {
    match motion {
        VimMotion::Character(motion) => match motion {
            CharacterMotion::Left => model.vim_move_cursors(
                |buffer, head| {
                    vim_line_bounded_destination(buffer, head, count, Direction::Backward)
                },
                ctx,
            ),
            CharacterMotion::Right => model.vim_move_cursors(
                |buffer, head| {
                    vim_line_bounded_destination(buffer, head, count, Direction::Forward)
                },
                ctx,
            ),
            CharacterMotion::WrappingLeft => {
                model.vim_move_across_lines(count, Direction::Backward, ctx)
            }
            CharacterMotion::WrappingRight => {
                model.vim_move_across_lines(count, Direction::Forward, ctx)
            }
            CharacterMotion::Up => model.vim_move_vertically(count, Direction::Backward, ctx),
            CharacterMotion::Down => model.vim_move_vertically(count, Direction::Forward, ctx),
        },
        VimMotion::Word(motion) => model.vim_move_cursors(
            |buffer, head| Some(vim_word_destination(buffer, head, motion, count)),
            ctx,
        ),
        VimMotion::Line(LineMotion::Start) => model.vim_move_cursors(vim_line_start, ctx),
        VimMotion::Line(LineMotion::FirstNonWhitespace) => {
            model.vim_move_cursors(vim_line_first_nonwhitespace, ctx)
        }
        VimMotion::Line(LineMotion::End) => {
            // Only `$` takes its count as a number of lines: `2$` is the end of the next line.
            model.vim_move_vertically(count.saturating_sub(1), Direction::Forward, ctx);
            model.vim_move_cursors(vim_line_end, ctx);
        }
        VimMotion::FirstNonWhitespace(motion) => {
            match motion {
                FirstNonWhitespaceMotion::Up => {
                    model.vim_move_vertically(count, Direction::Backward, ctx)
                }
                FirstNonWhitespaceMotion::Down => {
                    model.vim_move_vertically(count, Direction::Forward, ctx)
                }
                FirstNonWhitespaceMotion::DownMinusOne => {
                    model.vim_move_vertically(count.saturating_sub(1), Direction::Forward, ctx)
                }
            }
            model.vim_move_cursors(vim_line_first_nonwhitespace, ctx);
        }
        VimMotion::FindChar(motion) => model.vim_move_cursors(
            |buffer, head| vim_find_char_destination(buffer, head, motion, count, false),
            ctx,
        ),
        VimMotion::Paragraph(direction) => model.vim_move_cursors(
            |buffer, head| Some(vim_paragraph_destination(buffer, head, *direction, count)),
            ctx,
        ),
        VimMotion::JumpToFirstLine => model.vim_jump_to_first_line(ctx),
        VimMotion::JumpToLastLine => model.vim_jump_to_last_line(ctx),
        VimMotion::JumpToLine(line_number) => model.vim_jump_to_line(*line_number, ctx),
        VimMotion::JumpToMatchingBracket => {
            model.vim_move_cursors(vim_matching_bracket_destination, ctx)
        }
        VimMotion::JumpToUnmatchedBracket(bracket) => model.vim_move_cursors(
            |buffer, head| vim_find_matching_bracket(buffer, bracket, head),
            ctx,
        ),
    }
}

/// Moves `count` characters from `head` without leaving its line (`h` and `l`).
pub fn vim_line_bounded_destination<B: VimTextBuffer + ?Sized>(
    buffer: &B,
    head: CharOffset,
    count: u32,
    direction: Direction,
) -> Option<CharOffset> {
    let point = buffer.to_point(head).ok()?;
    Some(match direction {
        Direction::Backward => {
            let distance = point.column.min(count) as usize;
            CharOffset::from(head.as_usize().saturating_sub(distance))
        }
        Direction::Forward => {
            let distance = buffer
                .vim_line_len(point.row)
                .saturating_sub(point.column)
                .min(count) as usize;
            (head + distance).min(buffer.vim_max_offset())
        }
    })
}

/// The `count`th word boundary from `head` (`w`, `b`, `e`, `ge` and their WORD variants), or `head`
/// when there is none.
fn vim_word_destination<B: VimTextBuffer + ?Sized>(
    buffer: &B,
    head: CharOffset,
    motion: &WordMotion,
    count: u32,
) -> CharOffset {
    vim_word_iterator_from_offset(
        head,
        buffer,
        motion.direction,
        motion.bound,
        motion.word_type,
    )
    .ok()
    .and_then(|boundaries| boundaries.take(count as usize).last())
    .unwrap_or(head)
}

/// The first column of the line containing `head` (`0`).
fn vim_line_start<B: VimTextBuffer + ?Sized>(buffer: &B, head: CharOffset) -> Option<CharOffset> {
    let row = buffer.to_point(head).ok()?.row;
    buffer.to_offset(Point::new(row, 0)).ok()
}

/// The position after the last character of the line containing `head` (`$`).
fn vim_line_end<B: VimTextBuffer + ?Sized>(buffer: &B, head: CharOffset) -> Option<CharOffset> {
    let row = buffer.to_point(head).ok()?.row;
    buffer
        .to_offset(Point::new(row, buffer.vim_line_len(row)))
        .ok()
}

/// The first non-whitespace character of the line containing `head` (`^`), or the line start when
/// the line is blank.
fn vim_line_first_nonwhitespace<B: VimTextBuffer + ?Sized>(
    buffer: &B,
    head: CharOffset,
) -> Option<CharOffset> {
    let row = buffer.to_point(head).ok()?.row;
    let line_start = buffer.to_offset(Point::new(row, 0)).ok()?;
    let column = buffer
        .chars_at(line_start)
        .ok()?
        .take(buffer.vim_line_len(row) as usize)
        .position(|c| !c.is_whitespace())
        .unwrap_or(0);
    Some(line_start + column)
}

/// The destination of an `f`, `F`, `t`, or `T` motion from `head`, if the character is found on its
/// line. `keep_selection` has the meaning documented on [`vim_find_char_on_line`].
pub fn vim_find_char_destination<B: VimTextBuffer + ?Sized>(
    buffer: &B,
    head: CharOffset,
    motion: &FindCharMotion,
    occurrence_count: u32,
    keep_selection: bool,
) -> Option<CharOffset> {
    let point = buffer.to_point(head).ok()?;
    let line_start = buffer.to_offset(Point::new(point.row, 0)).ok()?;
    let line: String = buffer
        .chars_at(line_start)
        .ok()?
        .take(buffer.vim_line_len(point.row) as usize)
        .collect();
    let column = vim_find_char_on_line(
        &line,
        point.column as usize,
        motion,
        occurrence_count,
        keep_selection,
    )?;
    buffer.to_offset(Point::new(point.row, column as u32)).ok()
}

/// The `count`th paragraph boundary from `head` (`{` and `}`), or the buffer bound when there are
/// fewer boundaries than that.
pub fn vim_paragraph_destination<B: VimTextBuffer + ?Sized>(
    buffer: &B,
    head: CharOffset,
    direction: Direction,
    count: u32,
) -> CharOffset {
    (0..count).fold(head, |offset, _| match direction {
        Direction::Forward => {
            find_next_paragraph_end(buffer, offset).unwrap_or_else(|| buffer.vim_max_offset())
        }
        Direction::Backward => {
            find_previous_paragraph_start(buffer, offset).unwrap_or_else(|| buffer.vim_min_offset())
        }
    })
}

/// The bracket matching the first bracket at or after `head` on its line (`%`).
pub fn vim_matching_bracket_destination<B: VimTextBuffer + ?Sized>(
    buffer: &B,
    head: CharOffset,
) -> Option<CharOffset> {
    // Vim only considers brackets on the cursor's line as the starting point of the search.
    let (distance, bracket) = buffer
        .chars_at(head)
        .ok()?
        .take_while(|c| *c != '\n')
        .enumerate()
        .find_map(|(i, c)| Some((i, BracketChar::try_from(c).ok()?)))?;
    vim_find_matching_bracket(buffer, &bracket, head + distance)
}

#[cfg(test)]
#[path = "navigation_tests.rs"]
mod tests;
