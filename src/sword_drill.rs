use diesel::prelude::*;
use diesel::result::Error;
use diesel::sql_types::{Integer, Text};
use diesel::sqlite::Sqlite;
use regex::Regex;

use models::*;
use reference::Reference;
use BiblersError;

sql_function!(
    /// Represents the [`highlight` function](https://sqlite.org/fts5.html#the_highlight_function)
    /// from SQLite's FTS5 extension.
    fn highlight(table_name: Text, column_index: Integer, prefix: Text, suffix: Text) -> Text;
);

pub fn verses<C>(reference: &Reference, conn: &C) -> Result<(Book, Vec<Verse>), BiblersError>
where
    C: Connection<Backend = Sqlite>,
{
    use schema::book_abbreviations as ba;
    use schema::books as b;
    use schema::verses as v;

    // Get the book
    let (book, _) = b::table
        .inner_join(ba::table)
        .filter(ba::abbreviation.eq(reference.book.to_lowercase()))
        .first::<(Book, BookAbbreviation)>(conn)
        .map_err(|e| match e {
            Error::NotFound => BiblersError::BookNotFound {
                book: reference.book.to_owned(),
            },
            e => BiblersError::DatabaseError { root_cause: e },
        })?;

    let mut query = v::table
        .filter(v::book.eq(book.id))
        .filter(v::chapter.eq(reference.chapter))
        .into_boxed();

    if let Some(ref verses) = reference.verses {
        query = query.filter(v::verse.between(verses.start, verses.end));
    }

    query
        .order_by((v::chapter.asc(), v::verse.asc()))
        .load(conn)
        .and_then(|verses| Ok((book, verses)))
        .map_err(|e| BiblersError::DatabaseError { root_cause: e })
}

pub fn book<C>(book_name: &str, conn: &C) -> Result<(Book, Vec<i32>), BiblersError>
where
    C: Connection<Backend = Sqlite>,
{
    use schema::book_abbreviations as ba;
    use schema::books as b;

    let (book, _) = b::table
        .inner_join(ba::table)
        .filter(ba::abbreviation.eq(book_name.to_lowercase()))
        .order_by(b::id)
        .first::<(Book, BookAbbreviation)>(conn)
        .map_err(|e| match e {
            Error::NotFound => BiblersError::BookNotFound {
                book: book_name.to_owned(),
            },
            e => BiblersError::DatabaseError { root_cause: e },
        })?;
    let chapters: Vec<i32> = (1..=book.chapter_count).collect();

    Ok((book, chapters))
}

pub fn all_books<C>(conn: &C) -> Result<Vec<Book>, BiblersError>
where
    C: Connection<Backend = Sqlite>,
{
    use schema::books::dsl::*;

    books
        .order_by(id)
        .load(conn)
        .map_err(|e| BiblersError::DatabaseError { root_cause: e })
}

const SEARCH_RESULT_LIMIT: i64 = 15;

pub fn search<C>(query: &str, conn: &C) -> Result<Vec<(VerseFTS, Book)>, BiblersError>
where
    C: Connection<Backend = Sqlite>,
{
    use schema::books;
    use schema::verses_fts;

    lazy_static! {
        static ref ALPHA_NUM: Regex = Regex::new(r"[^a-zA-Z ]+").unwrap();
    }

    let had_quote = query.contains('"');

    // Replace all characters that aren't alpha or space
    let mut query = ALPHA_NUM.replace_all(query, "").to_string();

    // Don't even try to run the query if there are no characters
    if query.is_empty() {
        return Ok(vec![]);
    }

    // Add back quotes safely if it had a quote before, and was removed
    // This makes FTS5 query the string as a phrase.
    query = if had_quote {
        format!("\"{}\"", query)
    } else {
        query
    };

    verses_fts::table
        .inner_join(books::table.on(books::id.eq(verses_fts::book)))
        .select((
            (
                verses_fts::book,
                verses_fts::chapter,
                verses_fts::verse,
                highlight(verses_fts::text, 3, "<em>", "</em>"),
                verses_fts::rank,
            ),
            (
                books::id,
                books::name,
                books::chapter_count,
                books::testament,
            ),
        )).filter(verses_fts::text.eq(format!("{}*", query)))
        .order_by(verses_fts::rank)
        .limit(SEARCH_RESULT_LIMIT)
        .load::<(VerseFTS, Book)>(conn)
        .map_err(|e| BiblersError::DatabaseError { root_cause: e })
}
