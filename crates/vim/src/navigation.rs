use warpui_core::{Entity, ModelContext};

use crate::vim::{
    BracketChar, CharacterMotion, Direction, FindCharMotion, FirstNonWhitespaceMotion, LineMotion,
    VimMotion, WordMotion,
};

/// Cursor movements that Vim motions resolve to, implemented by an editor model. Each method
/// moves every cursor to its destination and collapses any selection onto it.
pub trait VimNavigation: Entity {
    /// Moves `count` characters. With `stop_at_line_boundary`, movement ends at the start or end
    /// of the current line; otherwise it continues onto adjacent lines.
    fn vim_nav_horizontal(
        &mut self,
        count: u32,
        direction: Direction,
        stop_at_line_boundary: bool,
        ctx: &mut ModelContext<Self>,
    );

    /// Moves `count` logical lines, keeping the goal column.
    fn vim_nav_vertical(&mut self, count: u32, direction: Direction, ctx: &mut ModelContext<Self>);

    fn vim_nav_line_start(&mut self, ctx: &mut ModelContext<Self>);

    fn vim_nav_line_end(&mut self, ctx: &mut ModelContext<Self>);

    fn vim_nav_first_nonwhitespace(&mut self, ctx: &mut ModelContext<Self>);

    fn vim_nav_word(&mut self, count: u32, motion: &WordMotion, ctx: &mut ModelContext<Self>);

    fn vim_nav_find_char(
        &mut self,
        occurrence_count: u32,
        motion: &FindCharMotion,
        ctx: &mut ModelContext<Self>,
    );

    fn vim_nav_paragraph(&mut self, count: u32, direction: Direction, ctx: &mut ModelContext<Self>);

    fn vim_nav_first_line(&mut self, ctx: &mut ModelContext<Self>);

    fn vim_nav_last_line(&mut self, ctx: &mut ModelContext<Self>);

    /// `line_number` is 1-based and clamped to the buffer.
    fn vim_nav_line_number(&mut self, line_number: u32, ctx: &mut ModelContext<Self>);

    fn vim_nav_matching_bracket(&mut self, ctx: &mut ModelContext<Self>);

    fn vim_nav_unmatched_bracket(&mut self, bracket: &BracketChar, ctx: &mut ModelContext<Self>);
}

/// Moves the cursors of `model` according to a normal-mode Vim `motion` repeated `count` times.
pub fn apply_vim_motion<M: VimNavigation>(
    model: &mut M,
    count: u32,
    motion: &VimMotion,
    ctx: &mut ModelContext<M>,
) {
    match motion {
        VimMotion::Character(motion) => match motion {
            CharacterMotion::Left => {
                model.vim_nav_horizontal(count, Direction::Backward, true, ctx)
            }
            CharacterMotion::Right => {
                model.vim_nav_horizontal(count, Direction::Forward, true, ctx)
            }
            CharacterMotion::WrappingLeft => {
                model.vim_nav_horizontal(count, Direction::Backward, false, ctx)
            }
            CharacterMotion::WrappingRight => {
                model.vim_nav_horizontal(count, Direction::Forward, false, ctx)
            }
            CharacterMotion::Up => model.vim_nav_vertical(count, Direction::Backward, ctx),
            CharacterMotion::Down => model.vim_nav_vertical(count, Direction::Forward, ctx),
        },
        VimMotion::Word(motion) => model.vim_nav_word(count, motion, ctx),
        VimMotion::Line(motion) => match motion {
            LineMotion::Start => model.vim_nav_line_start(ctx),
            LineMotion::FirstNonWhitespace => model.vim_nav_first_nonwhitespace(ctx),
            // Of the line motions, only `$` takes a count: `n$` ends on the (n-1)th line below.
            LineMotion::End => {
                model.vim_nav_vertical(count.saturating_sub(1), Direction::Forward, ctx);
                model.vim_nav_line_end(ctx);
            }
        },
        VimMotion::FirstNonWhitespace(motion) => {
            match motion {
                FirstNonWhitespaceMotion::Up => {
                    model.vim_nav_vertical(count, Direction::Backward, ctx)
                }
                FirstNonWhitespaceMotion::Down => {
                    model.vim_nav_vertical(count, Direction::Forward, ctx)
                }
                FirstNonWhitespaceMotion::DownMinusOne => {
                    model.vim_nav_vertical(count.saturating_sub(1), Direction::Forward, ctx)
                }
            }
            model.vim_nav_first_nonwhitespace(ctx);
        }
        VimMotion::FindChar(motion) => model.vim_nav_find_char(count, motion, ctx),
        VimMotion::Paragraph(direction) => model.vim_nav_paragraph(count, *direction, ctx),
        VimMotion::JumpToFirstLine => model.vim_nav_first_line(ctx),
        VimMotion::JumpToLastLine => model.vim_nav_last_line(ctx),
        VimMotion::JumpToLine(line_number) => model.vim_nav_line_number(*line_number, ctx),
        VimMotion::JumpToMatchingBracket => model.vim_nav_matching_bracket(ctx),
        VimMotion::JumpToUnmatchedBracket(bracket) => model.vim_nav_unmatched_bracket(bracket, ctx),
    }
}
