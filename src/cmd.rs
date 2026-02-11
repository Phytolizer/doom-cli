#[derive(Debug, Clone)]
pub(crate) struct CommandLine {
    lines: Vec<Line>,
}

#[derive(Debug, Clone)]
pub(crate) struct Line {
    words: Vec<String>,
}

impl CommandLine {
    pub fn new() -> Self {
        Self { lines: vec![] }
    }

    pub fn push_line(&mut self, line: Line) {
        self.lines.push(line);
    }

    pub fn iter_words(&self) -> impl Iterator<Item = &str> {
        self.lines.iter().flat_map(|line| line.iter())
    }

    pub fn iter_lines(&self) -> impl Iterator<Item = &Line> {
        self.lines.iter()
    }
}

const INDENTATION_WIDTH: usize = 4;

impl Line {
    fn indent_word(word: impl AsRef<str>, indentation_depth: usize) -> String {
        let mut indented_word =
            String::with_capacity(INDENTATION_WIDTH * indentation_depth + word.as_ref().len());
        for _ in 0..(indentation_depth * INDENTATION_WIDTH) {
            indented_word.push(' ');
        }
        indented_word.push_str(word.as_ref());
        indented_word
    }

    pub fn from_words(words: &[impl AsRef<str>], indentation_depth: usize) -> Self {
        Self {
            words: words
                .iter()
                .next()
                .map(|first| Self::indent_word(first, indentation_depth))
                .into_iter()
                .chain(words.iter().skip(1).map(|w| w.as_ref().to_string()))
                .collect(),
        }
    }

    pub fn from_word(word: impl AsRef<str>, indentation_depth: usize) -> Self {
        Self {
            words: vec![Self::indent_word(word, indentation_depth)],
        }
    }

    pub fn iter(&'_ self) -> LineIterator<'_> {
        LineIterator {
            line: self,
            index: 0,
        }
    }
}

pub(crate) struct LineIterator<'l> {
    line: &'l Line,
    index: usize,
}

impl<'l> Iterator for LineIterator<'l> {
    type Item = &'l str;

    fn nth(&mut self, n: usize) -> Option<Self::Item> {
        self.line.words.get(n).map(|s| s.as_str())
    }

    fn fold<B, F>(self, mut init: B, mut f: F) -> B
    where
        Self: Sized,
        F: FnMut(B, Self::Item) -> B,
    {
        for n in self {
            init = f(init, n);
        }
        init
    }

    fn next(&mut self) -> Option<Self::Item> {
        let value = self.line.words.get(self.index);
        self.index += 1;
        value.map(String::as_str)
    }
}
