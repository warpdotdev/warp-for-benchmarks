use vim::vim::{
    BracketChar, CharacterMotion, Direction, FindCharMotion, FirstNonWhitespaceMotion, LineMotion,
    WordMotion,
};
use warp_editor::model::CoreEditorModel;
use warp_editor::selection::TextDirection;
use warpui_core::ModelContext;

use super::model::{CodeEditorModel, LineBound};

pub fn navigate_char(
    model: &mut CodeEditorModel,
    count: u32,
    character_motion: &CharacterMotion,
    ctx: &mut ModelContext<CodeEditorModel>,
) {
    match character_motion {
        CharacterMotion::Right => {
            model.vim_move_horizontal_by_offset(count, &Direction::Forward, false, true, ctx);
        }
        CharacterMotion::Up => {
            model.vim_move_vertical_by_offset(count, TextDirection::Backwards, false, ctx);
        }
        CharacterMotion::Down => {
            model.vim_move_vertical_by_offset(count, TextDirection::Forwards, false, ctx);
        }
        CharacterMotion::Left => {
            model.vim_move_horizontal_by_offset(count, &Direction::Backward, false, true, ctx);
        }
        CharacterMotion::WrappingLeft => {
            model.vim_move_horizontal_by_offset(count, &Direction::Backward, false, false, ctx);
        }
        CharacterMotion::WrappingRight => {
            model.vim_move_horizontal_by_offset(count, &Direction::Forward, false, false, ctx);
        }
    }
}

pub fn navigate_word(
    model: &mut CodeEditorModel,
    count: u32,
    word_motion: &WordMotion,
    ctx: &mut ModelContext<CodeEditorModel>,
) {
    let WordMotion {
        direction,
        bound,
        word_type,
    } = word_motion;
    model.vim_navigate_word(*direction, *bound, *word_type, count, ctx);
}

pub fn navigate_line(
    model: &mut CodeEditorModel,
    line_count: u32,
    motion: &LineMotion,
    ctx: &mut ModelContext<CodeEditorModel>,
) {
    match motion {
        LineMotion::Start => model.vim_move_to_line_bound(LineBound::Start, false, ctx),
        LineMotion::FirstNonWhitespace => model.vim_move_to_first_nonwhitespace(false, ctx),
        LineMotion::End => {
            model.vim_move_vertical_by_offset(
                line_count.saturating_sub(1),
                TextDirection::Forwards,
                false,
                ctx,
            );
            model.vim_move_to_line_bound(LineBound::End, false, ctx);
        }
    }
}

pub fn first_nonwhitespace_motion(
    model: &mut CodeEditorModel,
    count: u32,
    motion: &FirstNonWhitespaceMotion,
    ctx: &mut ModelContext<CodeEditorModel>,
) {
    match motion {
        FirstNonWhitespaceMotion::Up => {
            model.vim_move_vertical_by_offset(count, TextDirection::Backwards, false, ctx);
        }
        FirstNonWhitespaceMotion::Down => {
            model.vim_move_vertical_by_offset(count, TextDirection::Forwards, false, ctx);
        }
        FirstNonWhitespaceMotion::DownMinusOne => {
            model.vim_move_vertical_by_offset(count - 1, TextDirection::Forwards, false, ctx);
        }
    }
    model.vim_move_to_first_nonwhitespace(false, ctx);
}

pub fn find_char(
    model: &mut CodeEditorModel,
    occurrence_count: u32,
    find_char_motion: &FindCharMotion,
    ctx: &mut ModelContext<CodeEditorModel>,
) {
    model.vim_find_char(false, occurrence_count, find_char_motion, ctx);
}

pub fn navigate_paragraph(
    model: &mut CodeEditorModel,
    count: u32,
    direction: &Direction,
    ctx: &mut ModelContext<CodeEditorModel>,
) {
    model.vim_move_by_paragraph(count, direction, false, ctx);
}

pub fn jump_to_first_line(
    model: &mut CodeEditorModel,
    column: Option<usize>,
    ctx: &mut ModelContext<CodeEditorModel>,
) {
    model.jump_to_line_column(0, column, ctx);
}

pub fn jump_to_last_line(model: &mut CodeEditorModel, ctx: &mut ModelContext<CodeEditorModel>) {
    model.vim_move_to_last_line(ctx);
}

pub fn jump_to_line(
    model: &mut CodeEditorModel,
    line_number: u32,
    column: Option<usize>,
    ctx: &mut ModelContext<CodeEditorModel>,
) {
    let buffer = model.content().as_ref(ctx);
    let max_row = buffer.max_point().row;
    let row = line_number.max(1).min(max_row);
    model.jump_to_line_column(row as usize, column, ctx);
}

pub fn jump_to_matching_bracket(
    model: &mut CodeEditorModel,
    ctx: &mut ModelContext<CodeEditorModel>,
) {
    model.vim_jump_to_matching_bracket(false, ctx);
}

pub fn jump_to_unmatched_bracket(
    model: &mut CodeEditorModel,
    bracket: &BracketChar,
    ctx: &mut ModelContext<CodeEditorModel>,
) {
    model.vim_jump_to_unmatched_bracket(bracket, false, ctx);
}
