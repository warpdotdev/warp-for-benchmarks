use warpui_core::ModelContext;

use crate::vim::{
    BracketChar, CharacterMotion, Direction, FindCharMotion, FirstNonWhitespaceMotion, LineMotion,
    VimMotion, WordMotion,
};

/// Cursor movements an editor model provides so that [`apply_vim_motion`] can map every
/// [`VimMotion`] onto them. Each movement collapses every selection to a cursor at its destination.
pub trait VimMotionTarget {
    /// Moves `count` characters in `direction`, stopping at the line boundary unless `wrap_lines`.
    fn vim_motion_chars(
        &mut self,
        count: u32,
        direction: Direction,
        wrap_lines: bool,
        ctx: &mut ModelContext<Self>,
    );
    /// Moves `count` lines in `direction`, keeping the goal column.
    fn vim_motion_lines(&mut self, count: u32, direction: Direction, ctx: &mut ModelContext<Self>);
    fn vim_motion_line_start(&mut self, ctx: &mut ModelContext<Self>);
    fn vim_motion_line_end(&mut self, ctx: &mut ModelContext<Self>);
    fn vim_motion_first_nonwhitespace(&mut self, ctx: &mut ModelContext<Self>);
    fn vim_motion_words(&mut self, count: u32, motion: &WordMotion, ctx: &mut ModelContext<Self>);
    fn vim_motion_find_char(
        &mut self,
        count: u32,
        motion: &FindCharMotion,
        ctx: &mut ModelContext<Self>,
    );
    fn vim_motion_paragraphs(
        &mut self,
        count: u32,
        direction: Direction,
        ctx: &mut ModelContext<Self>,
    );
    fn vim_motion_first_line(&mut self, ctx: &mut ModelContext<Self>);
    fn vim_motion_last_line(&mut self, ctx: &mut ModelContext<Self>);
    /// Moves to the 1-indexed `line_number`, clamped to the buffer.
    fn vim_motion_line_number(&mut self, line_number: u32, ctx: &mut ModelContext<Self>);
    fn vim_motion_matching_bracket(&mut self, ctx: &mut ModelContext<Self>);
    fn vim_motion_unmatched_bracket(&mut self, bracket: &BracketChar, ctx: &mut ModelContext<Self>);
}

/// Moves the cursor(s) of `target` according to `motion`, repeated `count` times for motions that
/// accept a count.
pub fn apply_vim_motion<T: VimMotionTarget>(
    target: &mut T,
    count: u32,
    motion: &VimMotion,
    ctx: &mut ModelContext<T>,
) {
    match motion {
        VimMotion::Character(motion) => match motion {
            CharacterMotion::Left => target.vim_motion_chars(count, Direction::Backward, false, ctx),
            CharacterMotion::Right => target.vim_motion_chars(count, Direction::Forward, false, ctx),
            CharacterMotion::WrappingLeft => {
                target.vim_motion_chars(count, Direction::Backward, true, ctx)
            }
            CharacterMotion::WrappingRight => {
                target.vim_motion_chars(count, Direction::Forward, true, ctx)
            }
            CharacterMotion::Up => target.vim_motion_lines(count, Direction::Backward, ctx),
            CharacterMotion::Down => target.vim_motion_lines(count, Direction::Forward, ctx),
        },
        VimMotion::Word(motion) => target.vim_motion_words(count, motion, ctx),
        VimMotion::Line(motion) => match motion {
            LineMotion::Start => target.vim_motion_line_start(ctx),
            LineMotion::FirstNonWhitespace => target.vim_motion_first_nonwhitespace(ctx),
            LineMotion::End => {
                target.vim_motion_lines(count.saturating_sub(1), Direction::Forward, ctx);
                target.vim_motion_line_end(ctx);
            }
        },
        VimMotion::FirstNonWhitespace(motion) => {
            match motion {
                FirstNonWhitespaceMotion::Up => {
                    target.vim_motion_lines(count, Direction::Backward, ctx)
                }
                FirstNonWhitespaceMotion::Down => {
                    target.vim_motion_lines(count, Direction::Forward, ctx)
                }
                FirstNonWhitespaceMotion::DownMinusOne => {
                    target.vim_motion_lines(count.saturating_sub(1), Direction::Forward, ctx)
                }
            }
            target.vim_motion_first_nonwhitespace(ctx);
        }
        VimMotion::FindChar(motion) => target.vim_motion_find_char(count, motion, ctx),
        VimMotion::Paragraph(direction) => target.vim_motion_paragraphs(count, *direction, ctx),
        VimMotion::JumpToFirstLine => target.vim_motion_first_line(ctx),
        VimMotion::JumpToLastLine => target.vim_motion_last_line(ctx),
        VimMotion::JumpToLine(line_number) => target.vim_motion_line_number(*line_number, ctx),
        VimMotion::JumpToMatchingBracket => target.vim_motion_matching_bracket(ctx),
        VimMotion::JumpToUnmatchedBracket(bracket) => {
            target.vim_motion_unmatched_bracket(bracket, ctx)
        }
    }
}
