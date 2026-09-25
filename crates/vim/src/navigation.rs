use warpui_core::ModelContext;

use crate::vim::{
    BracketChar, CharacterMotion, Direction, FindCharMotion, FirstNonWhitespaceMotion, LineMotion,
    WordMotion,
};

pub trait VimNavigation: Sized {
    fn move_horizontal(
        &mut self,
        count: u32,
        direction: Direction,
        stop_at_line_boundary: bool,
        ctx: &mut ModelContext<Self>,
    );
    fn move_vertical(&mut self, count: u32, direction: Direction, ctx: &mut ModelContext<Self>);
    fn move_to_line_start(&mut self, ctx: &mut ModelContext<Self>);
    fn move_to_first_nonwhitespace(&mut self, ctx: &mut ModelContext<Self>);
    fn move_to_line_end(&mut self, ctx: &mut ModelContext<Self>);
    fn move_word(&mut self, count: u32, motion: &WordMotion, ctx: &mut ModelContext<Self>);
    fn move_to_char(&mut self, count: u32, motion: &FindCharMotion, ctx: &mut ModelContext<Self>);
    fn move_paragraph(&mut self, count: u32, direction: Direction, ctx: &mut ModelContext<Self>);
    fn move_to_first_line(&mut self, ctx: &mut ModelContext<Self>);
    fn move_to_last_line(&mut self, ctx: &mut ModelContext<Self>);
    fn move_to_line(&mut self, line_number: u32, ctx: &mut ModelContext<Self>);
    fn move_to_matching_bracket(&mut self, ctx: &mut ModelContext<Self>);
    fn move_to_unmatched_bracket(&mut self, bracket: &BracketChar, ctx: &mut ModelContext<Self>);

    fn navigate_char(
        &mut self,
        count: u32,
        motion: &CharacterMotion,
        wrap_lines: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        match motion {
            CharacterMotion::Left => self.move_horizontal(count, Direction::Backward, true, ctx),
            CharacterMotion::Right => self.move_horizontal(count, Direction::Forward, true, ctx),
            CharacterMotion::WrappingLeft => {
                self.move_horizontal(count, Direction::Backward, !wrap_lines, ctx)
            }
            CharacterMotion::WrappingRight => {
                self.move_horizontal(count, Direction::Forward, !wrap_lines, ctx)
            }
            CharacterMotion::Up => self.move_vertical(count, Direction::Backward, ctx),
            CharacterMotion::Down => self.move_vertical(count, Direction::Forward, ctx),
        }
    }

    fn navigate_line(&mut self, count: u32, motion: &LineMotion, ctx: &mut ModelContext<Self>) {
        match motion {
            LineMotion::Start => self.move_to_line_start(ctx),
            LineMotion::FirstNonWhitespace => self.move_to_first_nonwhitespace(ctx),
            LineMotion::End => {
                self.move_vertical(count.saturating_sub(1), Direction::Forward, ctx);
                self.move_to_line_end(ctx);
            }
        }
    }

    fn navigate_first_nonwhitespace(
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
        self.move_vertical(count, direction, ctx);
        self.move_to_first_nonwhitespace(ctx);
    }
}
