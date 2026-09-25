use vim::vim::{
    BracketChar, CharacterMotion, Direction, FindCharMotion, FirstNonWhitespaceMotion, LineMotion,
    WordMotion,
};
use warp_editor::model::CoreEditorModel;
use warp_editor::selection::TextDirection;
use warpui::{Entity, ModelContext};

use crate::code::editor::model::{CodeEditorModel, LineBound};

pub trait VimNavigationModel: Entity + Sized {
    fn move_horizontal(
        &mut self,
        count: u32,
        direction: &Direction,
        wrapping: bool,
        ctx: &mut ModelContext<Self>,
    );
    fn move_vertical(&mut self, count: u32, direction: TextDirection, ctx: &mut ModelContext<Self>);
    fn move_word(&mut self, count: u32, motion: &WordMotion, ctx: &mut ModelContext<Self>);
    fn move_line_start(&mut self, ctx: &mut ModelContext<Self>);
    fn move_line_end(&mut self, ctx: &mut ModelContext<Self>);
    fn move_first_nonwhitespace(&mut self, ctx: &mut ModelContext<Self>);
    fn find_char(&mut self, count: u32, motion: &FindCharMotion, ctx: &mut ModelContext<Self>);
    fn move_paragraph(&mut self, count: u32, direction: &Direction, ctx: &mut ModelContext<Self>);
    fn jump_first_line(&mut self, column: Option<usize>, ctx: &mut ModelContext<Self>);
    fn jump_last_line(&mut self, ctx: &mut ModelContext<Self>);
    fn jump_line(&mut self, line_number: u32, column: Option<usize>, ctx: &mut ModelContext<Self>);
    fn jump_matching_bracket(&mut self, ctx: &mut ModelContext<Self>);
    fn jump_unmatched_bracket(&mut self, bracket: &BracketChar, ctx: &mut ModelContext<Self>);

    fn navigate_char(
        &mut self,
        count: u32,
        motion: &CharacterMotion,
        ctx: &mut ModelContext<Self>,
    ) {
        match motion {
            CharacterMotion::Left => self.move_horizontal(count, &Direction::Backward, false, ctx),
            CharacterMotion::Right => self.move_horizontal(count, &Direction::Forward, false, ctx),
            CharacterMotion::WrappingLeft => {
                self.move_horizontal(count, &Direction::Backward, true, ctx)
            }
            CharacterMotion::WrappingRight => {
                self.move_horizontal(count, &Direction::Forward, true, ctx)
            }
            CharacterMotion::Up => self.move_vertical(count, TextDirection::Backwards, ctx),
            CharacterMotion::Down => self.move_vertical(count, TextDirection::Forwards, ctx),
        }
    }

    fn navigate_line(&mut self, count: u32, motion: &LineMotion, ctx: &mut ModelContext<Self>) {
        match motion {
            LineMotion::Start => self.move_line_start(ctx),
            LineMotion::FirstNonWhitespace => self.move_first_nonwhitespace(ctx),
            LineMotion::End => {
                self.move_vertical(count.saturating_sub(1), TextDirection::Forwards, ctx);
                self.move_line_end(ctx);
            }
        }
    }

    fn navigate_first_nonwhitespace(
        &mut self,
        count: u32,
        motion: &FirstNonWhitespaceMotion,
        ctx: &mut ModelContext<Self>,
    ) {
        match motion {
            FirstNonWhitespaceMotion::Up => {
                self.move_vertical(count, TextDirection::Backwards, ctx)
            }
            FirstNonWhitespaceMotion::Down => {
                self.move_vertical(count, TextDirection::Forwards, ctx)
            }
            FirstNonWhitespaceMotion::DownMinusOne => {
                self.move_vertical(count - 1, TextDirection::Forwards, ctx)
            }
        }
        self.move_first_nonwhitespace(ctx);
    }
}

impl VimNavigationModel for CodeEditorModel {
    fn move_horizontal(
        &mut self,
        count: u32,
        direction: &Direction,
        wrapping: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        self.vim_move_horizontal_by_offset(count, direction, false, !wrapping, ctx);
    }

    fn move_vertical(
        &mut self,
        count: u32,
        direction: TextDirection,
        ctx: &mut ModelContext<Self>,
    ) {
        self.vim_move_vertical_by_offset(count, direction, false, ctx);
    }

    fn move_word(&mut self, count: u32, motion: &WordMotion, ctx: &mut ModelContext<Self>) {
        self.vim_navigate_word(motion.direction, motion.bound, motion.word_type, count, ctx);
    }

    fn move_line_start(&mut self, ctx: &mut ModelContext<Self>) {
        self.vim_move_to_line_bound(LineBound::Start, false, ctx);
    }

    fn move_line_end(&mut self, ctx: &mut ModelContext<Self>) {
        self.vim_move_to_line_bound(LineBound::End, false, ctx);
    }

    fn move_first_nonwhitespace(&mut self, ctx: &mut ModelContext<Self>) {
        self.vim_move_to_first_nonwhitespace(false, ctx);
    }

    fn find_char(&mut self, count: u32, motion: &FindCharMotion, ctx: &mut ModelContext<Self>) {
        self.vim_find_char(false, count, motion, ctx);
    }

    fn move_paragraph(&mut self, count: u32, direction: &Direction, ctx: &mut ModelContext<Self>) {
        self.vim_move_by_paragraph(count, direction, false, ctx);
    }

    fn jump_first_line(&mut self, column: Option<usize>, ctx: &mut ModelContext<Self>) {
        self.jump_to_line_column(0, column, ctx);
    }

    fn jump_last_line(&mut self, ctx: &mut ModelContext<Self>) {
        self.vim_move_to_last_line(ctx);
    }

    fn jump_line(&mut self, line_number: u32, column: Option<usize>, ctx: &mut ModelContext<Self>) {
        let max_row = self.content().as_ref(ctx).max_point().row;
        let row = line_number.max(1).min(max_row);
        self.jump_to_line_column(row as usize, column, ctx);
    }

    fn jump_matching_bracket(&mut self, ctx: &mut ModelContext<Self>) {
        self.vim_jump_to_matching_bracket(false, ctx);
    }

    fn jump_unmatched_bracket(&mut self, bracket: &BracketChar, ctx: &mut ModelContext<Self>) {
        self.vim_jump_to_unmatched_bracket(bracket, false, ctx);
    }
}
