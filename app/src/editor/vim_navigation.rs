use vim::vim::{
    BracketChar, CharacterMotion, Direction, FindCharMotion, LineMotion, VimMotion, WordMotion,
};
use warp_editor::model::CoreEditorModel;
use warp_editor::selection::TextDirection;
use warpui::{Entity, ModelContext};

use super::Point;
use super::view::model::EditorModel;
use crate::code::editor::model::{CodeEditorModel, LineBound};

#[derive(Clone, Copy)]
pub struct VimNavigationOptions {
    first_line_column: Option<usize>,
    last_line_column: Option<usize>,
    numbered_line_column: Option<usize>,
    minimum_line_number: u32,
    line_number_offset: u32,
    wrap_character_motions: bool,
}

impl VimNavigationOptions {
    pub const EDITOR_VIEW: Self = Self {
        first_line_column: Some(0),
        last_line_column: Some(0),
        numbered_line_column: Some(0),
        minimum_line_number: 0,
        line_number_offset: 1,
        wrap_character_motions: true,
    };

    pub const CODE_EDITOR_VIEW: Self = Self {
        first_line_column: None,
        last_line_column: None,
        numbered_line_column: None,
        minimum_line_number: 1,
        line_number_offset: 0,
        wrap_character_motions: true,
    };

    pub const TUI_INPUT_VIEW: Self = Self {
        first_line_column: Some(0),
        last_line_column: None,
        numbered_line_column: Some(0),
        minimum_line_number: 1,
        line_number_offset: 0,
        wrap_character_motions: false,
    };
}

pub trait VimNavigation: Entity {
    fn vim_move_horizontal(
        &mut self,
        count: u32,
        direction: Direction,
        wrapping: bool,
        ctx: &mut ModelContext<Self>,
    );

    fn vim_move_vertical(&mut self, count: u32, direction: Direction, ctx: &mut ModelContext<Self>);

    fn vim_move_word(&mut self, count: u32, motion: &WordMotion, ctx: &mut ModelContext<Self>);

    fn move_to_line_bound(&mut self, motion: LineMotion, ctx: &mut ModelContext<Self>);

    fn move_to_first_nonwhitespace(&mut self, ctx: &mut ModelContext<Self>);

    fn find_char_destination(
        &mut self,
        count: u32,
        motion: &FindCharMotion,
        ctx: &mut ModelContext<Self>,
    );

    fn vim_move_paragraph(
        &mut self,
        count: u32,
        direction: Direction,
        ctx: &mut ModelContext<Self>,
    );

    fn vim_jump_to_first_line(
        &mut self,
        options: VimNavigationOptions,
        ctx: &mut ModelContext<Self>,
    );

    fn vim_jump_to_last_line(
        &mut self,
        options: VimNavigationOptions,
        ctx: &mut ModelContext<Self>,
    );

    fn vim_jump_to_line(
        &mut self,
        line_number: u32,
        options: VimNavigationOptions,
        ctx: &mut ModelContext<Self>,
    );

    fn jump_to_matching_bracket(&mut self, ctx: &mut ModelContext<Self>);

    fn jump_to_unmatched_bracket(&mut self, bracket: &BracketChar, ctx: &mut ModelContext<Self>);

    fn navigate_vim(
        &mut self,
        count: u32,
        motion: &VimMotion,
        options: VimNavigationOptions,
        ctx: &mut ModelContext<Self>,
    ) {
        match motion {
            VimMotion::Character(CharacterMotion::Left) => {
                self.vim_move_horizontal(count, Direction::Backward, false, ctx);
            }
            VimMotion::Character(CharacterMotion::Right) => {
                self.vim_move_horizontal(count, Direction::Forward, false, ctx);
            }
            VimMotion::Character(CharacterMotion::WrappingLeft) => {
                self.vim_move_horizontal(
                    count,
                    Direction::Backward,
                    options.wrap_character_motions,
                    ctx,
                );
            }
            VimMotion::Character(CharacterMotion::WrappingRight) => {
                self.vim_move_horizontal(
                    count,
                    Direction::Forward,
                    options.wrap_character_motions,
                    ctx,
                );
            }
            VimMotion::Character(CharacterMotion::Up) => {
                self.vim_move_vertical(count, Direction::Backward, ctx);
            }
            VimMotion::Character(CharacterMotion::Down) => {
                self.vim_move_vertical(count, Direction::Forward, ctx);
            }
            VimMotion::Word(motion) => self.vim_move_word(count, motion, ctx),
            VimMotion::Line(LineMotion::End) => {
                self.vim_move_vertical(count.saturating_sub(1), Direction::Forward, ctx);
                self.move_to_line_bound(LineMotion::End, ctx);
            }
            VimMotion::Line(motion) => self.move_to_line_bound(*motion, ctx),
            VimMotion::FirstNonWhitespace(motion) => {
                let (count, direction) = match motion {
                    vim::vim::FirstNonWhitespaceMotion::Up => (count, Direction::Backward),
                    vim::vim::FirstNonWhitespaceMotion::Down => (count, Direction::Forward),
                    vim::vim::FirstNonWhitespaceMotion::DownMinusOne => {
                        (count - 1, Direction::Forward)
                    }
                };
                self.vim_move_vertical(count, direction, ctx);
                self.move_to_first_nonwhitespace(ctx);
            }
            VimMotion::FindChar(motion) => self.find_char_destination(count, motion, ctx),
            VimMotion::Paragraph(direction) => self.vim_move_paragraph(count, *direction, ctx),
            VimMotion::JumpToFirstLine => self.vim_jump_to_first_line(options, ctx),
            VimMotion::JumpToLastLine => self.vim_jump_to_last_line(options, ctx),
            VimMotion::JumpToLine(line_number) => {
                self.vim_jump_to_line(*line_number, options, ctx);
            }
            VimMotion::JumpToMatchingBracket => self.jump_to_matching_bracket(ctx),
            VimMotion::JumpToUnmatchedBracket(bracket) => {
                self.jump_to_unmatched_bracket(bracket, ctx);
            }
        }
    }
}

impl VimNavigation for EditorModel {
    fn vim_move_horizontal(
        &mut self,
        count: u32,
        direction: Direction,
        wrapping: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        if wrapping {
            self.move_cursor_ignoring_newlines(count, &direction, false, ctx);
        } else {
            self.move_cursors_by_offset(count, &direction, false, true, ctx);
        }
    }

    fn vim_move_vertical(
        &mut self,
        count: u32,
        direction: Direction,
        ctx: &mut ModelContext<Self>,
    ) {
        match direction {
            Direction::Backward => self.move_up_by_offset(count, ctx),
            Direction::Forward => self.move_down_by_offset(count, ctx),
        }
    }

    fn vim_move_word(&mut self, count: u32, motion: &WordMotion, ctx: &mut ModelContext<Self>) {
        self.vim_navigate_word(motion.direction, motion.bound, motion.word_type, count, ctx);
    }

    fn move_to_line_bound(&mut self, motion: LineMotion, ctx: &mut ModelContext<Self>) {
        match motion {
            LineMotion::Start => self.cursor_line_start(false, ctx),
            LineMotion::FirstNonWhitespace => self.cursor_line_start_non_whitespace(false, ctx),
            LineMotion::End => self.cursor_line_end(false, ctx),
        }
    }

    fn move_to_first_nonwhitespace(&mut self, ctx: &mut ModelContext<Self>) {
        self.cursor_line_start_non_whitespace(false, ctx);
    }

    fn find_char_destination(
        &mut self,
        count: u32,
        motion: &FindCharMotion,
        ctx: &mut ModelContext<Self>,
    ) {
        self.vim_find_char(false, count, motion, ctx);
    }

    fn vim_move_paragraph(
        &mut self,
        count: u32,
        direction: Direction,
        ctx: &mut ModelContext<Self>,
    ) {
        self.vim_move_by_paragraph(count, &direction, false, ctx);
    }

    fn vim_jump_to_first_line(
        &mut self,
        _options: VimNavigationOptions,
        ctx: &mut ModelContext<Self>,
    ) {
        self.reset_selections_to_point(&Point::new(0, 0), ctx);
    }

    fn vim_jump_to_last_line(
        &mut self,
        _options: VimNavigationOptions,
        ctx: &mut ModelContext<Self>,
    ) {
        self.move_to_buffer_end(false, ctx);
        self.cursor_line_start(false, ctx);
    }

    fn vim_jump_to_line(
        &mut self,
        line_number: u32,
        options: VimNavigationOptions,
        ctx: &mut ModelContext<Self>,
    ) {
        let max_row = self.buffer(ctx).max_point().row;
        let row = line_number
            .max(options.minimum_line_number)
            .saturating_sub(options.line_number_offset)
            .min(max_row);
        self.reset_selections_to_point(&Point::new(row, 0), ctx);
    }

    fn jump_to_matching_bracket(&mut self, ctx: &mut ModelContext<Self>) {
        self.vim_move_cursor_to_matching_bracket(false, ctx);
    }

    fn jump_to_unmatched_bracket(&mut self, bracket: &BracketChar, ctx: &mut ModelContext<Self>) {
        self.vim_move_cursor_to_unmatched_bracket(bracket, false, ctx);
    }
}

impl VimNavigation for CodeEditorModel {
    fn vim_move_horizontal(
        &mut self,
        count: u32,
        direction: Direction,
        wrapping: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        self.vim_move_horizontal_by_offset(count, &direction, false, !wrapping, ctx);
    }

    fn vim_move_vertical(
        &mut self,
        count: u32,
        direction: Direction,
        ctx: &mut ModelContext<Self>,
    ) {
        let direction = match direction {
            Direction::Backward => TextDirection::Backwards,
            Direction::Forward => TextDirection::Forwards,
        };
        self.vim_move_vertical_by_offset(count, direction, false, ctx);
    }

    fn vim_move_word(&mut self, count: u32, motion: &WordMotion, ctx: &mut ModelContext<Self>) {
        self.vim_navigate_word(motion.direction, motion.bound, motion.word_type, count, ctx);
    }

    fn move_to_line_bound(&mut self, motion: LineMotion, ctx: &mut ModelContext<Self>) {
        match motion {
            LineMotion::Start => self.vim_move_to_line_bound(LineBound::Start, false, ctx),
            LineMotion::FirstNonWhitespace => self.vim_move_to_first_nonwhitespace(false, ctx),
            LineMotion::End => self.vim_move_to_line_bound(LineBound::End, false, ctx),
        }
    }

    fn move_to_first_nonwhitespace(&mut self, ctx: &mut ModelContext<Self>) {
        self.vim_move_to_first_nonwhitespace(false, ctx);
    }

    fn find_char_destination(
        &mut self,
        count: u32,
        motion: &FindCharMotion,
        ctx: &mut ModelContext<Self>,
    ) {
        self.vim_find_char(false, count, motion, ctx);
    }

    fn vim_move_paragraph(
        &mut self,
        count: u32,
        direction: Direction,
        ctx: &mut ModelContext<Self>,
    ) {
        self.vim_move_by_paragraph(count, &direction, false, ctx);
    }

    fn vim_jump_to_first_line(
        &mut self,
        options: VimNavigationOptions,
        ctx: &mut ModelContext<Self>,
    ) {
        self.jump_to_line_column(0, options.first_line_column, ctx);
    }

    fn vim_jump_to_last_line(
        &mut self,
        options: VimNavigationOptions,
        ctx: &mut ModelContext<Self>,
    ) {
        if options.last_line_column.is_none() {
            self.vim_move_to_last_line(ctx);
        } else {
            let max_row = self.content().as_ref(ctx).max_point().row;
            self.jump_to_line_column(max_row as usize, options.last_line_column, ctx);
        }
    }

    fn vim_jump_to_line(
        &mut self,
        line_number: u32,
        options: VimNavigationOptions,
        ctx: &mut ModelContext<Self>,
    ) {
        let max_row = self.content().as_ref(ctx).max_point().row;
        let row = line_number
            .max(options.minimum_line_number)
            .saturating_sub(options.line_number_offset)
            .min(max_row);
        self.jump_to_line_column(row as usize, options.numbered_line_column, ctx);
    }

    fn jump_to_matching_bracket(&mut self, ctx: &mut ModelContext<Self>) {
        self.vim_jump_to_matching_bracket(false, ctx);
    }

    fn jump_to_unmatched_bracket(&mut self, bracket: &BracketChar, ctx: &mut ModelContext<Self>) {
        self.vim_jump_to_unmatched_bracket(bracket, false, ctx);
    }
}
