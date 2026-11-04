//! CSV tables whose header is the row struct. [`row!`] names each column after its field, so
//! a column cannot drift from its value.
use super::price::Usd;
use chrono::{DateTime, SecondsFormat, Utc};

/// A value that fills one cell. `None`, and a number that is not finite, leave the cell empty,
/// which Python and R read as missing.
pub trait Cell {
    fn cell(&self) -> String;
}

impl Cell for String {
    fn cell(&self) -> String {
        self.clone()
    }
}

impl Cell for &str {
    fn cell(&self) -> String {
        (*self).to_owned()
    }
}

impl Cell for bool {
    fn cell(&self) -> String {
        self.to_string()
    }
}

impl Cell for f64 {
    fn cell(&self) -> String {
        if self.is_finite() {
            self.to_string()
        } else {
            String::new()
        }
    }
}

impl Cell for Usd {
    fn cell(&self) -> String {
        self.to_string()
    }
}

impl Cell for DateTime<Utc> {
    fn cell(&self) -> String {
        self.to_rfc3339_opts(SecondsFormat::AutoSi, true)
    }
}

impl<T: Cell> Cell for Option<T> {
    fn cell(&self) -> String {
        self.as_ref().map_or_else(String::new, Cell::cell)
    }
}

macro_rules! integer_cells {
    ($($type:ty),*) => {$(
        impl Cell for $type {
            fn cell(&self) -> String {
                self.to_string()
            }
        }
    )*};
}

integer_cells!(u8, i32, u32, u64, usize);

/// Adjacent columns: a [`Cell`] is one column named after its field, and a row nested in a
/// row is its own columns, spliced in where the field is.
pub trait Columns {
    fn names(field: &'static str, names: &mut Vec<&'static str>);
    fn cells(&self, cells: &mut Vec<String>);
}

impl<T: Cell> Columns for T {
    fn names(field: &'static str, names: &mut Vec<&'static str>) {
        names.push(field);
    }

    fn cells(&self, cells: &mut Vec<String>) {
        cells.push(self.cell());
    }
}

/// Declares a row struct whose fields, in order, are its columns.
macro_rules! row {
    (
        $(#[$meta:meta])*
        pub struct $name:ident {
            $($(#[$field_meta:meta])* pub $field:ident: $type:ty,)*
        }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Debug, PartialEq)]
        pub struct $name {
            $($(#[$field_meta])* pub $field: $type,)*
        }

        impl $crate::report::csv::Columns for $name {
            fn names(_: &'static str, names: &mut Vec<&'static str>) {
                $(<$type as $crate::report::csv::Columns>::names(stringify!($field), names);)*
            }

            fn cells(&self, cells: &mut Vec<String>) {
                $($crate::report::csv::Columns::cells(&self.$field, cells);)*
            }
        }
    };
}

pub(crate) use row;

/// `rows` as CSV under their header, which is there even when no row is.
pub fn encode<R: Columns>(rows: &[R]) -> String {
    let mut header = Vec::new();
    R::names("", &mut header);
    let mut text = String::new();
    line(&mut text, header.iter().copied());
    for row in rows {
        let mut cells = Vec::with_capacity(header.len());
        row.cells(&mut cells);
        line(&mut text, cells.iter().map(String::as_str));
    }
    text
}

/// RFC 4180: a cell holding a comma, a quote or a line break is quoted, its quotes doubled.
fn line<'a>(text: &mut String, cells: impl IntoIterator<Item = &'a str>) {
    for (index, cell) in cells.into_iter().enumerate() {
        if index > 0 {
            text.push(',');
        }
        if cell.contains([',', '"', '\n', '\r']) {
            text.push('"');
            text.push_str(&cell.replace('"', "\"\""));
            text.push('"');
        } else {
            text.push_str(cell);
        }
    }
    text.push('\n');
}

#[cfg(test)]
mod tests {
    use super::*;

    row! {
        pub struct Inner {
            pub low_s: f64,
            pub high_s: Option<f64>,
        }
    }

    row! {
        pub struct Outer {
            pub name: String,
            pub spread: Inner,
            pub count: u64,
            pub done: bool,
        }
    }

    #[test]
    fn the_header_follows_the_fields_and_splices_in_nested_rows() {
        let rows = [Outer {
            name: "a".into(),
            spread: Inner {
                low_s: 0.25,
                high_s: Some(1.5),
            },
            count: 3,
            done: true,
        }];
        assert_eq!(
            encode(&rows),
            "name,low_s,high_s,count,done\na,0.25,1.5,3,true\n"
        );
    }

    #[test]
    fn an_empty_table_is_its_header() {
        assert_eq!(encode::<Outer>(&[]), "name,low_s,high_s,count,done\n");
    }

    #[test]
    fn missing_values_and_non_finite_numbers_leave_the_cell_empty() {
        let rows = [Outer {
            name: String::new(),
            spread: Inner {
                low_s: f64::NAN,
                high_s: None,
            },
            count: 0,
            done: false,
        }];
        assert_eq!(encode(&rows).lines().nth(1), Some(",,,0,false"));
    }

    #[test]
    fn cells_with_separators_quotes_or_line_breaks_are_quoted() {
        let row = |name: &str| Outer {
            name: name.into(),
            spread: Inner {
                low_s: 1.0,
                high_s: None,
            },
            count: 1,
            done: false,
        };
        let rows = [
            row("a,b"),
            row("say \"hi\""),
            row("two\nlines"),
            row("plain"),
        ];
        let body = encode(&rows);
        let body = body.split_once('\n').unwrap().1;
        assert_eq!(
            body,
            "\"a,b\",1,,1,false\n\"say \"\"hi\"\"\",1,,1,false\n\"two\nlines\",1,,1,false\n\
             plain,1,,1,false\n"
        );
    }

    #[test]
    fn times_are_rfc_3339_in_utc_and_numbers_keep_their_precision() {
        let time: DateTime<Utc> = "2026-11-04T03:14:41.487316966Z".parse().unwrap();
        assert_eq!(time.cell(), "2026-11-04T03:14:41.487316966Z");
        assert_eq!(59.768284981_f64.cell(), "59.768284981");
        assert_eq!(1e-7_f64.cell(), "0.0000001");
        assert_eq!(f64::INFINITY.cell(), "");
    }
}
