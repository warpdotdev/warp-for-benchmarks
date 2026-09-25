mod matching_brackets;
pub use matching_brackets::vim_find_matching_bracket;

mod paragraph_iterator;
pub use paragraph_iterator::{find_next_paragraph_end, find_previous_paragraph_start};
pub mod register;

mod text_objects;
pub use text_objects::*;

mod word_iterator;
pub use word_iterator::vim_word_iterator_from_offset;

mod find_char;
pub use find_char::vim_find_char_on_line;
use vim::{CharacterMotion, Direction, FirstNonWhitespaceMotion, LineMotion, WordMotion};
use warpui_core::{Entity, ModelContext};

pub trait VimNavigation: Entity {
    fn vim_move_horizontal(
        &mut self,
        count: u32,
        direction: Direction,
        stop_at_line_boundary: bool,
        ctx: &mut ModelContext<Self>,
    );
    fn vim_move_vertical(&mut self, count: u32, direction: Direction, ctx: &mut ModelContext<Self>);
    fn vim_move_word(&mut self, count: u32, motion: &WordMotion, ctx: &mut ModelContext<Self>);
    fn vim_move_line_start(&mut self, ctx: &mut ModelContext<Self>);
    fn vim_move_line_end(&mut self, ctx: &mut ModelContext<Self>);
    fn vim_move_first_nonwhitespace(&mut self, ctx: &mut ModelContext<Self>);

    fn vim_navigate_character(
        &mut self,
        count: u32,
        motion: &CharacterMotion,
        wrapping_motions_cross_lines: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        match motion {
            CharacterMotion::Left => {
                self.vim_move_horizontal(count, Direction::Backward, true, ctx)
            }
            CharacterMotion::Right => {
                self.vim_move_horizontal(count, Direction::Forward, true, ctx)
            }
            CharacterMotion::WrappingLeft => self.vim_move_horizontal(
                count,
                Direction::Backward,
                !wrapping_motions_cross_lines,
                ctx,
            ),
            CharacterMotion::WrappingRight => self.vim_move_horizontal(
                count,
                Direction::Forward,
                !wrapping_motions_cross_lines,
                ctx,
            ),
            CharacterMotion::Up => self.vim_move_vertical(count, Direction::Backward, ctx),
            CharacterMotion::Down => self.vim_move_vertical(count, Direction::Forward, ctx),
        }
    }

    fn vim_navigate_word_motion(
        &mut self,
        count: u32,
        motion: &WordMotion,
        ctx: &mut ModelContext<Self>,
    ) {
        self.vim_move_word(count, motion, ctx);
    }

    fn vim_navigate_line_motion(
        &mut self,
        count: u32,
        motion: &LineMotion,
        ctx: &mut ModelContext<Self>,
    ) {
        match motion {
            LineMotion::Start => self.vim_move_line_start(ctx),
            LineMotion::FirstNonWhitespace => self.vim_move_first_nonwhitespace(ctx),
            LineMotion::End => {
                self.vim_move_vertical(count.saturating_sub(1), Direction::Forward, ctx);
                self.vim_move_line_end(ctx);
            }
        }
    }

    fn vim_navigate_first_nonwhitespace(
        &mut self,
        count: u32,
        motion: &FirstNonWhitespaceMotion,
        ctx: &mut ModelContext<Self>,
    ) {
        let (count, direction) = match motion {
            FirstNonWhitespaceMotion::Up => (count, Direction::Backward),
            FirstNonWhitespaceMotion::Down => (count, Direction::Forward),
            FirstNonWhitespaceMotion::DownMinusOne => (count - 1, Direction::Forward),
        };
        self.vim_move_vertical(count, direction, ctx);
        self.vim_move_first_nonwhitespace(ctx);
    }
}

pub mod vim;
