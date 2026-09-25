use anyhow::{Result, anyhow};
use string_offset::CharOffset;
use warpui_core::text::TextBuffer;
use warpui_core::text::point::Point;

use super::{
    LineColumn, LineNumbering, VimNavigationConfig, VimNavigationState, WrappingMotionBehavior,
    vim_motion_destination,
};
use crate::vim::{
    BracketChar, CharacterMotion, Direction, FindCharDestination, FindCharMotion,
    FirstNonWhitespaceMotion, LineMotion, VimMotion, WordBound, WordMotion, WordType,
};

struct TestBuffer(String);

impl TextBuffer for TestBuffer {
    type Chars<'a> = std::vec::IntoIter<char>;
    type CharsReverse<'a> = std::vec::IntoIter<char>;

    fn chars_at(&self, offset: CharOffset) -> Result<Self::Chars<'_>> {
        if offset.as_usize() > self.0.chars().count() {
            return Err(anyhow!("offset out of bounds"));
        }
        Ok(self
            .0
            .chars()
            .skip(offset.as_usize())
            .collect::<Vec<_>>()
            .into_iter())
    }

    fn chars_rev_at(&self, offset: CharOffset) -> Result<Self::CharsReverse<'_>> {
        if offset.as_usize() > self.0.chars().count() {
            return Err(anyhow!("offset out of bounds"));
        }
        let mut chars = self.0.chars().take(offset.as_usize()).collect::<Vec<_>>();
        chars.reverse();
        Ok(chars.into_iter())
    }

    fn to_point(&self, offset: CharOffset) -> Result<Point> {
        if offset.as_usize() > self.0.chars().count() {
            return Err(anyhow!("offset out of bounds"));
        }
        let prefix: String = self.0.chars().take(offset.as_usize()).collect();
        let row = prefix.chars().filter(|c| *c == '\n').count() as u32;
        let column = prefix
            .rsplit_once('\n')
            .map_or(prefix.chars().count(), |(_, line)| line.chars().count())
            as u32;
        Ok(Point::new(row, column))
    }

    fn to_offset(&self, point: Point) -> Result<CharOffset> {
        let mut row = 0;
        let mut column = 0;
        for (offset, c) in self.0.chars().enumerate() {
            if row == point.row && column == point.column {
                return Ok(CharOffset::from(offset));
            }
            if c == '\n' {
                row += 1;
                column = 0;
            } else {
                column += 1;
            }
        }
        if row == point.row && column == point.column {
            Ok(CharOffset::from(self.0.chars().count()))
        } else {
            Err(anyhow!("point out of bounds"))
        }
    }
}

fn config() -> VimNavigationConfig {
    VimNavigationConfig {
        wrapping_motion_behavior: WrappingMotionBehavior::IgnoreNewlines,
        paragraph_start_fallback: 0,
        first_line_column: LineColumn::Start,
        last_line_column: LineColumn::Start,
        numbered_line_column: LineColumn::Start,
        line_numbering: LineNumbering::OneBased,
    }
}

fn destination(buffer: &str, offset: usize, count: u32, motion: VimMotion) -> VimNavigationState {
    vim_motion_destination(
        &TestBuffer(buffer.to_owned()),
        VimNavigationState {
            offset: CharOffset::from(offset),
            goal_column: None,
        },
        count,
        &motion,
        config(),
    )
}

#[test]
fn computes_character_and_vertical_destinations() {
    let state = destination(
        "abc\ndefgh\nij",
        2,
        2,
        VimMotion::Character(CharacterMotion::Down),
    );
    assert_eq!(state.offset, CharOffset::from(12));
    assert_eq!(state.goal_column, Some(2));

    let state = vim_motion_destination(
        &TestBuffer("abc\ndefgh\nij".to_owned()),
        state,
        1,
        &VimMotion::Character(CharacterMotion::Up),
        config(),
    );
    assert_eq!(state.offset, CharOffset::from(6));
    assert_eq!(state.goal_column, Some(2));

    assert_eq!(
        destination(
            "abc\ndef",
            2,
            1,
            VimMotion::Character(CharacterMotion::WrappingRight),
        )
        .offset,
        CharOffset::from(3)
    );
    assert_eq!(
        destination(
            "a\nb",
            1,
            1,
            VimMotion::Character(CharacterMotion::WrappingRight),
        )
        .offset,
        CharOffset::from(2)
    );
    assert_eq!(
        destination(
            "a\nb",
            2,
            1,
            VimMotion::Character(CharacterMotion::WrappingLeft),
        )
        .offset,
        CharOffset::from(1)
    );
}

#[test]
fn computes_word_line_and_first_nonwhitespace_destinations() {
    let word = VimMotion::Word(WordMotion::new(
        Direction::Forward,
        WordBound::Start,
        WordType::Default,
    ));
    assert_eq!(
        destination("one two\n  three", 0, 2, word).offset,
        CharOffset::from(10)
    );
    assert_eq!(
        destination("one\n  two\nthree", 0, 2, VimMotion::Line(LineMotion::End),).offset,
        CharOffset::from(9)
    );
    assert_eq!(
        destination(
            "one\n  two\n    three",
            0,
            2,
            VimMotion::FirstNonWhitespace(FirstNonWhitespaceMotion::Down),
        )
        .offset,
        CharOffset::from(14)
    );
}

#[test]
fn computes_find_paragraph_line_and_bracket_destinations() {
    let find = FindCharMotion {
        direction: Direction::Forward,
        destination: FindCharDestination::AtChar,
        is_repetition: false,
        c: 'c',
    };
    assert_eq!(
        destination("abc abc", 0, 2, VimMotion::FindChar(find)).offset,
        CharOffset::from(6)
    );
    assert_eq!(
        destination(
            "one\n\n two\n\nthree",
            0,
            2,
            VimMotion::Paragraph(Direction::Forward),
        )
        .offset,
        CharOffset::from(10)
    );
    assert_eq!(
        destination("one\ntwo\nthree", 6, 1, VimMotion::JumpToLastLine).offset,
        CharOffset::from(8)
    );
    assert_eq!(
        destination("a(b[c]d)e", 0, 1, VimMotion::JumpToMatchingBracket,).offset,
        CharOffset::from(7)
    );
    assert_eq!(
        destination(
            "a(b[c]d)e",
            4,
            1,
            VimMotion::JumpToUnmatchedBracket(BracketChar::try_from('[').expect("opening bracket")),
        )
        .offset,
        CharOffset::from(5)
    );
}
