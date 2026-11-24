use crate::column::{ColumnData, ColumnarTable, StringColumn, MISSING_VALID};
use crate::sas_format::{format_missing_tag, format_numeric_value};
use crate::types::VariableMeta;

/// VS Code-style search options for dataset cell-value search.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CellSearchOptions {
    pub match_case: bool,
    pub whole_word: bool,
    pub use_regex: bool,
}

/// Compiled matcher shared by full-table search and visible-viewport highlighting.
#[derive(Debug, Clone)]
pub struct CellSearchMatcher {
    query: String,
    query_lower: String,
    query_is_ascii: bool,
    options: CellSearchOptions,
    parsed_numeric: Option<f64>,
    can_match_plain_numeric: bool,
    regex: Option<MiniRegex>,
}

impl CellSearchMatcher {
    pub fn compile(raw_query: &str, options: CellSearchOptions) -> Result<Self, String> {
        let query = raw_query.to_string();
        let query_lower = query.to_lowercase();
        let query_is_ascii = query.is_ascii();
        let parsed_numeric = query.trim().parse::<f64>().ok().filter(|v| v.is_finite());

        let regex = if options.use_regex {
            let pattern = if options.whole_word {
                format!(r"\b(?:{})\b", query)
            } else {
                query.clone()
            };
            Some(MiniRegex::compile(&pattern, options.match_case)?)
        } else {
            None
        };

        let can_match_plain_numeric = if options.use_regex {
            true
        } else {
            query_could_match_plain_numeric(&query)
        };

        Ok(Self {
            query,
            query_lower,
            query_is_ascii,
            options,
            parsed_numeric,
            can_match_plain_numeric,
            regex,
        })
    }

    pub fn options(&self) -> CellSearchOptions {
        self.options
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    /// Match a plain text string (e.g. a SAS character cell or formatted date/missing string).
    pub fn matches_str(&self, haystack: &str) -> bool {
        if self.query.is_empty() || haystack.is_empty() {
            return false;
        }
        if let Some(re) = &self.regex {
            return re.is_match(haystack);
        }
        if self.options.whole_word {
            self.matches_whole_word(haystack)
        } else if self.options.match_case {
            haystack.contains(&self.query)
        } else if self.query_is_ascii {
            contains_ascii_ci(haystack.as_bytes(), self.query_lower.as_bytes())
        } else {
            haystack.to_lowercase().contains(&self.query_lower)
        }
    }

    /// Match a numeric cell (handles plain numbers, formatted decimals, dates, and SAS missing tags).
    pub fn matches_numeric(&self, val: f64, missing_tag: u8, meta: &VariableMeta) -> bool {
        if self.query.is_empty() {
            return false;
        }
        if missing_tag != MISSING_VALID {
            let tag_str = format_missing_tag(missing_tag);
            return self.matches_str(&tag_str);
        }
        if meta.date_kind.is_temporal() {
            let formatted = format_numeric_value(val, meta);
            return self.matches_str(&formatted);
        }

        if let Some(re) = &self.regex {
            let formatted = format_numeric_value(val, meta);
            return re.is_match(&formatted);
        }

        if !self.can_match_plain_numeric {
            return false;
        }

        // Exact numeric equivalence (e.g. searching "65" matches "65.00", "65.0" matches "65")
        if let Some(q_num) = self.parsed_numeric {
            let tol = 1e-10 * val.abs().max(1.0);
            if (val - q_num).abs() <= tol {
                return true;
            }
            if self.options.whole_word {
                // In whole-word mode for a plain numeric cell when query is a number,
                // e.g. searching "65" must NOT match "65.3" or "-65" or "165".
                return false;
            }
        }

        let formatted = format_numeric_value(val, meta);
        self.matches_str(&formatted)
    }

    /// Match any cell in `table` at `(orig_row, orig_col)`.
    pub fn matches_cell(&self, table: &ColumnarTable, orig_row: usize, orig_col: usize) -> bool {
        let Some(col) = table.columns.get(orig_col) else {
            return false;
        };
        let Some(meta) = table.variables.get(orig_col) else {
            return false;
        };
        match col {
            ColumnData::String(sc) => self.matches_str(sc.get_str(orig_row)),
            ColumnData::Numeric(nc) => {
                self.matches_numeric(nc.values[orig_row], nc.missing[orig_row], meta)
            }
        }
    }

    fn matches_whole_word(&self, haystack: &str) -> bool {
        let hay_chars: Vec<char> = if self.options.match_case {
            haystack.chars().collect()
        } else {
            haystack.chars().flat_map(|c| c.to_lowercase()).collect()
        };
        let pat_chars: Vec<char> = if self.options.match_case {
            self.query.chars().collect()
        } else {
            self.query_lower.chars().collect()
        };
        let plen = pat_chars.len();
        if plen == 0 || plen > hay_chars.len() {
            return false;
        }
        for start in 0..=(hay_chars.len() - plen) {
            if hay_chars[start..start + plen] == pat_chars[..] {
                let end = start + plen;
                let left_ok = start == 0 || !is_word_char_at(&hay_chars, start - 1);
                let right_ok = end == hay_chars.len() || !is_word_char_at(&hay_chars, end);
                if left_ok && right_ok {
                    return true;
                }
            }
        }
        false
    }
}

/// Fast full-table cell search across `visible_rows` × `visible_cols`.
/// Returns `(matches_as_vis_row_and_orig_col, truncated)`.
pub fn search_table_cells(
    table: &ColumnarTable,
    visible_rows: &[u32],
    visible_cols: &[usize],
    matcher: &CellSearchMatcher,
    max_matches: usize,
) -> (Vec<(usize, usize)>, bool) {
    if matcher.query().is_empty() || visible_rows.is_empty() || visible_cols.is_empty() {
        return (Vec::new(), false);
    }

    enum ColPlan<'a> {
        Dict(&'a [u32], Vec<bool>),
        Flat(&'a StringColumn),
        NumericExactF64(&'a [f64], &'a [u8], f64),
        NumericGeneral(usize),
    }

    let plans: Vec<(usize, ColPlan<'_>)> = visible_cols
        .iter()
        .filter_map(|&c| {
            let col = table.columns.get(c)?;
            let meta = table.variables.get(c)?;
            let plan = match col {
                ColumnData::String(sc) => match sc {
                    StringColumn::Dict { dict, ids, .. } => {
                        let mask: Vec<bool> = dict
                            .iter()
                            .map(|s| matcher.matches_str(s))
                            .collect();
                        if !mask.iter().any(|&b| b) {
                            return None; // O(1) column skip!
                        }
                        ColPlan::Dict(ids, mask)
                    }
                    StringColumn::Flat { .. } => ColPlan::Flat(sc),
                },
                ColumnData::Numeric(nc) => {
                    if !meta.date_kind.is_temporal() && !matcher.options.use_regex {
                        if !matcher.can_match_plain_numeric {
                            return None; // O(1) skip plain numeric col for non-numeric text query!
                        }
                        if matcher.options.whole_word {
                            if let Some(q_num) = matcher.parsed_numeric {
                                return Some((c, ColPlan::NumericExactF64(&nc.values, &nc.missing, q_num)));
                            }
                        }
                    }
                    ColPlan::NumericGeneral(c)
                }
            };
            Some((c, plan))
        })
        .collect();

    if plans.is_empty() {
        return (Vec::new(), false);
    }

    let mut matches = Vec::new();
    let mut truncated = false;

    'outer: for (vis_r, &orig_u32) in visible_rows.iter().enumerate() {
        let orig_r = orig_u32 as usize;
        for &(col_idx, ref plan) in &plans {
            let hit = match plan {
                ColPlan::Dict(ids, mask) => mask[ids[orig_r] as usize],
                ColPlan::Flat(sc) => matcher.matches_str(sc.get_str(orig_r)),
                ColPlan::NumericExactF64(vals, miss, q_num) => {
                    if miss[orig_r] == MISSING_VALID {
                        let v = vals[orig_r];
                        (v - *q_num).abs() <= 1e-10 * v.abs().max(1.0)
                    } else {
                        false
                    }
                }
                ColPlan::NumericGeneral(c) => {
                    let meta = &table.variables[*c];
                    if let ColumnData::Numeric(nc) = &table.columns[*c] {
                        matcher.matches_numeric(nc.values[orig_r], nc.missing[orig_r], meta)
                    } else {
                        false
                    }
                }
            };
            if hit {
                matches.push((vis_r, col_idx));
                if matches.len() >= max_matches {
                    truncated = true;
                    break 'outer;
                }
            }
        }
    }

    (matches, truncated)
}

#[inline]
fn contains_ascii_ci(haystack: &[u8], needle_lower: &[u8]) -> bool {
    let nlen = needle_lower.len();
    if nlen == 0 || nlen > haystack.len() {
        return false;
    }
    let first = needle_lower[0];
    let last_start = haystack.len() - nlen;
    for i in 0..=last_start {
        if haystack[i].to_ascii_lowercase() == first {
            let mut ok = true;
            for j in 1..nlen {
                if haystack[i + j].to_ascii_lowercase() != needle_lower[j] {
                    ok = false;
                    break;
                }
            }
            if ok {
                return true;
            }
        }
    }
    false
}

/// Checks whether a non-regex query could possibly match ANY formatted plain numeric value
/// (`123`, `-45.67`, `1.2e-5`) or SAS missing value (`.`, `.A`-`.Z`, `._`).
fn query_could_match_plain_numeric(query: &str) -> bool {
    if query.is_empty() {
        return false;
    }
    // Check if query could match a SAS missing tag (`.`, `.A`-`.Z`, `._`, or single char `A`-`Z`/`_`)
    if query.len() <= 2 && query.is_ascii() {
        let bytes = query.as_bytes();
        if bytes.len() == 1 {
            let b = bytes[0];
            if b.is_ascii_alphanumeric() || b == b'.' || b == b'-' || b == b'+' || b == b'_' {
                return true;
            }
        } else if bytes.len() == 2 && bytes[0] == b'.' {
            let b = bytes[1];
            if b.is_ascii_alphanumeric() || b == b'_' {
                return true;
            }
        }
    }
    // Otherwise it can only match a formatted number: digits, `.`, `-`, `+`, `e`/`E`
    query.bytes().all(|b| {
        b.is_ascii_digit() || b == b'.' || b == b'-' || b == b'+' || b == b'e' || b == b'E'
    })
}

/// Word character helper: treats letters, digits, `_`, AND decimal points between two ASCII digits
/// (e.g. `.` inside `65.3`) as word characters so whole-word `"65"` does NOT falsely match `"65.3"`.
#[inline]
fn is_word_char_at(chars: &[char], idx: usize) -> bool {
    let c = chars[idx];
    if c.is_alphanumeric() || c == '_' {
        return true;
    }
    if c == '.'
        && idx > 0
        && idx + 1 < chars.len()
        && chars[idx - 1].is_ascii_digit()
        && chars[idx + 1].is_ascii_digit()
    {
        return true;
    }
    false
}

// ============================================================================
// Lightweight Zero-Dependency Thompson NFA Regex Engine (O(N*S), ReDoS-free)
// ============================================================================

#[derive(Debug, Clone)]
enum CharPred {
    Any,
    Literal(char),
    Digit(bool), // true = \d, false = \D
    Word(bool),  // true = \w, false = \W
    Space(bool), // true = \s, false = \S
    Class {
        negated: bool,
        items: Vec<ClassItem>,
    },
}

#[derive(Debug, Clone)]
enum ClassItem {
    Single(char),
    Range(char, char),
    Digit(bool),
    Word(bool),
    Space(bool),
}

impl CharPred {
    fn matches(&self, c: char, match_case: bool) -> bool {
        let ch = if match_case {
            c
        } else {
            c.to_lowercase().next().unwrap_or(c)
        };
        match self {
            CharPred::Any => true,
            CharPred::Literal(lit) => ch == *lit,
            CharPred::Digit(pos) => c.is_ascii_digit() == *pos,
            CharPred::Word(pos) => (c.is_alphanumeric() || c == '_') == *pos,
            CharPred::Space(pos) => c.is_whitespace() == *pos,
            CharPred::Class { negated, items } => {
                let hit = items.iter().any(|item| match item {
                    ClassItem::Single(s) => ch == *s,
                    ClassItem::Range(a, b) => ch >= *a && ch <= *b,
                    ClassItem::Digit(pos) => c.is_ascii_digit() == *pos,
                    ClassItem::Word(pos) => (c.is_alphanumeric() || c == '_') == *pos,
                    ClassItem::Space(pos) => c.is_whitespace() == *pos,
                });
                if *negated { !hit } else { hit }
            }
        }
    }
}

#[derive(Debug, Clone)]
enum NfaState {
    Pred(CharPred, usize),
    Split(usize, usize),
    Jump(usize),
    AssertStart(usize),
    AssertEnd(usize),
    AssertWordBoundary(bool, usize), // true = \b, false = \B
    Match,
}

#[derive(Debug, Clone)]
struct MiniRegex {
    states: Vec<NfaState>,
    start: usize,
    match_case: bool,
}

impl MiniRegex {
    fn compile(pattern: &str, match_case: bool) -> Result<Self, String> {
        let chars: Vec<char> = pattern.chars().collect();
        let mut parser = RegexParser {
            chars: &chars,
            pos: 0,
            match_case,
            states: Vec::new(),
        };
        // Reserve state 0 & 1 for unanchored prefix: Split(frag_start, 1), Any -> 0
        parser.states.push(NfaState::Jump(0)); // 0: placeholder
        parser.states.push(NfaState::Pred(CharPred::Any, 0)); // 1: consume any char and loop to 0

        let frag = parser.parse_alt()?;
        if parser.pos < chars.len() {
            return Err(format!("意外的字符 '{}'", chars[parser.pos]));
        }
        let match_state = parser.states.len();
        parser.states.push(NfaState::Match);
        parser.patch(frag.outs, match_state);
        parser.states[0] = NfaState::Split(frag.start, 1);

        Ok(Self {
            states: parser.states,
            start: 0,
            match_case,
        })
    }

    fn is_match(&self, haystack: &str) -> bool {
        let chars: Vec<char> = haystack.chars().collect();
        let n = self.states.len();
        let mut cur = Vec::with_capacity(16);
        let mut next = Vec::with_capacity(16);
        let mut seen = vec![0u32; n];
        let mut gen = 1u32;

        self.add_state(&chars, 0, self.start, &mut cur, &mut seen, gen);
        if cur.iter().any(|&s| matches!(self.states[s], NfaState::Match)) {
            return true;
        }

        for (i, &c) in chars.iter().enumerate() {
            next.clear();
            gen = gen.wrapping_add(1);
            if gen == 0 {
                seen.fill(0);
                gen = 1;
            }
            for &s in &cur {
                if let NfaState::Pred(ref pred, out) = self.states[s] {
                    if pred.matches(c, self.match_case) {
                        self.add_state(&chars, i + 1, out, &mut next, &mut seen, gen);
                    }
                }
            }
            std::mem::swap(&mut cur, &mut next);
            if cur.iter().any(|&s| matches!(self.states[s], NfaState::Match)) {
                return true;
            }
            if cur.is_empty() {
                break;
            }
        }
        false
    }

    fn add_state(
        &self,
        chars: &[char],
        pos: usize,
        state: usize,
        out_list: &mut Vec<usize>,
        seen: &mut [u32],
        gen: u32,
    ) {
        let mut stack = vec![state];
        while let Some(s) = stack.pop() {
            if seen[s] == gen {
                continue;
            }
            seen[s] = gen;
            match &self.states[s] {
                NfaState::Jump(a) => stack.push(*a),
                NfaState::Split(a, b) => {
                    stack.push(*b);
                    stack.push(*a);
                }
                NfaState::AssertStart(a) => {
                    if pos == 0 {
                        stack.push(*a);
                    }
                }
                NfaState::AssertEnd(a) => {
                    if pos == chars.len() {
                        stack.push(*a);
                    }
                }
                NfaState::AssertWordBoundary(want_boundary, a) => {
                    let prev_w = pos > 0 && is_word_char_at(chars, pos - 1);
                    let next_w = pos < chars.len() && is_word_char_at(chars, pos);
                    if (prev_w != next_w) == *want_boundary {
                        stack.push(*a);
                    }
                }
                NfaState::Pred(_, _) | NfaState::Match => {
                    out_list.push(s);
                }
            }
        }
    }
}

struct Frag {
    start: usize,
    outs: Vec<OutPtr>,
}

#[derive(Clone, Copy)]
enum OutPtr {
    Single(usize),
    SplitB(usize),
}

struct RegexParser<'a> {
    chars: &'a [char],
    pos: usize,
    match_case: bool,
    states: Vec<NfaState>,
}

impl<'a> RegexParser<'a> {
    fn fold_char(&self, c: char) -> char {
        if self.match_case {
            c
        } else {
            c.to_lowercase().next().unwrap_or(c)
        }
    }

    fn patch(&mut self, outs: Vec<OutPtr>, target: usize) {
        for ptr in outs {
            match ptr {
                OutPtr::Single(idx) => match &mut self.states[idx] {
                    NfaState::Pred(_, out)
                    | NfaState::Jump(out)
                    | NfaState::AssertStart(out)
                    | NfaState::AssertEnd(out)
                    | NfaState::AssertWordBoundary(_, out) => *out = target,
                    _ => {}
                },
                OutPtr::SplitB(idx) => {
                    if let NfaState::Split(_, b) = &mut self.states[idx] {
                        *b = target;
                    }
                }
            }
        }
    }

    fn empty_frag(&mut self) -> Frag {
        let s = self.states.len();
        self.states.push(NfaState::Jump(0));
        Frag {
            start: s,
            outs: vec![OutPtr::Single(s)],
        }
    }

    fn parse_alt(&mut self) -> Result<Frag, String> {
        let mut left = self.parse_concat()?;
        while self.pos < self.chars.len() && self.chars[self.pos] == '|' {
            self.pos += 1;
            let right = self.parse_concat()?;
            let s = self.states.len();
            self.states.push(NfaState::Split(left.start, right.start));
            let mut outs = left.outs;
            outs.extend(right.outs);
            left = Frag { start: s, outs };
        }
        Ok(left)
    }

    fn parse_concat(&mut self) -> Result<Frag, String> {
        let mut acc: Option<Frag> = None;
        while self.pos < self.chars.len() {
            let c = self.chars[self.pos];
            if c == '|' || c == ')' {
                break;
            }
            let atom = self.parse_quantified()?;
            acc = Some(match acc {
                None => atom,
                Some(prev) => {
                    self.patch(prev.outs, atom.start);
                    Frag {
                        start: prev.start,
                        outs: atom.outs,
                    }
                }
            });
        }
        Ok(acc.unwrap_or_else(|| self.empty_frag()))
    }

    fn parse_quantified(&mut self) -> Result<Frag, String> {
        let mut frag = self.parse_atom()?;
        while self.pos < self.chars.len() {
            match self.chars[self.pos] {
                '*' => {
                    self.pos += 1;
                    let s = self.states.len();
                    self.states.push(NfaState::Split(frag.start, 0));
                    self.patch(frag.outs, s);
                    frag = Frag {
                        start: s,
                        outs: vec![OutPtr::SplitB(s)],
                    };
                }
                '+' => {
                    self.pos += 1;
                    let s = self.states.len();
                    self.states.push(NfaState::Split(frag.start, 0));
                    self.patch(frag.outs, s);
                    frag = Frag {
                        start: frag.start,
                        outs: vec![OutPtr::SplitB(s)],
                    };
                }
                '?' => {
                    self.pos += 1;
                    let s = self.states.len();
                    self.states.push(NfaState::Split(frag.start, 0));
                    let mut outs = frag.outs;
                    outs.push(OutPtr::SplitB(s));
                    frag = Frag { start: s, outs };
                }
                _ => break,
            }
            // Optional lazy modifier `*?`, `+?`, `??` (same boolean match semantics)
            if self.pos < self.chars.len() && self.chars[self.pos] == '?' {
                self.pos += 1;
            }
        }
        Ok(frag)
    }

    fn parse_atom(&mut self) -> Result<Frag, String> {
        if self.pos >= self.chars.len() {
            return Err("正则表达式意外结束".to_string());
        }
        let c = self.chars[self.pos];
        match c {
            '(' => {
                self.pos += 1;
                // Optional non-capturing `(?:...)`
                if self.pos + 1 < self.chars.len()
                    && self.chars[self.pos] == '?'
                    && self.chars[self.pos + 1] == ':'
                {
                    self.pos += 2;
                }
                let inner = self.parse_alt()?;
                if self.pos >= self.chars.len() || self.chars[self.pos] != ')' {
                    return Err("缺少闭合括号 ')'".to_string());
                }
                self.pos += 1;
                Ok(inner)
            }
            '[' => self.parse_char_class(),
            '^' => {
                self.pos += 1;
                let s = self.states.len();
                self.states.push(NfaState::AssertStart(0));
                Ok(Frag {
                    start: s,
                    outs: vec![OutPtr::Single(s)],
                })
            }
            '$' => {
                self.pos += 1;
                let s = self.states.len();
                self.states.push(NfaState::AssertEnd(0));
                Ok(Frag {
                    start: s,
                    outs: vec![OutPtr::Single(s)],
                })
            }
            '.' => {
                self.pos += 1;
                let s = self.states.len();
                self.states.push(NfaState::Pred(CharPred::Any, 0));
                Ok(Frag {
                    start: s,
                    outs: vec![OutPtr::Single(s)],
                })
            }
            '\\' => {
                self.pos += 1;
                if self.pos >= self.chars.len() {
                    return Err("末尾存在未完成的转义 '\\'".to_string());
                }
                let esc = self.chars[self.pos];
                self.pos += 1;
                let state = match esc {
                    'b' => NfaState::AssertWordBoundary(true, 0),
                    'B' => NfaState::AssertWordBoundary(false, 0),
                    'd' => NfaState::Pred(CharPred::Digit(true), 0),
                    'D' => NfaState::Pred(CharPred::Digit(false), 0),
                    'w' => NfaState::Pred(CharPred::Word(true), 0),
                    'W' => NfaState::Pred(CharPred::Word(false), 0),
                    's' => NfaState::Pred(CharPred::Space(true), 0),
                    'S' => NfaState::Pred(CharPred::Space(false), 0),
                    't' => NfaState::Pred(CharPred::Literal('\t'), 0),
                    'n' => NfaState::Pred(CharPred::Literal('\n'), 0),
                    other => NfaState::Pred(CharPred::Literal(self.fold_char(other)), 0),
                };
                let s = self.states.len();
                self.states.push(state);
                Ok(Frag {
                    start: s,
                    outs: vec![OutPtr::Single(s)],
                })
            }
            '*' | '+' | '?' => Err(format!("量词 '{}' 缺少前置匹配项", c)),
            _ => {
                self.pos += 1;
                let s = self.states.len();
                self.states
                    .push(NfaState::Pred(CharPred::Literal(self.fold_char(c)), 0));
                Ok(Frag {
                    start: s,
                    outs: vec![OutPtr::Single(s)],
                })
            }
        }
    }

    fn parse_char_class(&mut self) -> Result<Frag, String> {
        self.pos += 1; // skip '['
        let negated = if self.pos < self.chars.len() && self.chars[self.pos] == '^' {
            self.pos += 1;
            true
        } else {
            false
        };

        let mut items = Vec::new();
        let mut first = true;
        while self.pos < self.chars.len() {
            let c = self.chars[self.pos];
            if c == ']' && !first {
                self.pos += 1;
                let s = self.states.len();
                self.states.push(NfaState::Pred(
                    CharPred::Class { negated, items },
                    0,
                ));
                return Ok(Frag {
                    start: s,
                    outs: vec![OutPtr::Single(s)],
                });
            }
            first = false;

            if c == '\\' {
                self.pos += 1;
                if self.pos >= self.chars.len() {
                    return Err("字符集 [...] 内存在未完成的转义".to_string());
                }
                let esc = self.chars[self.pos];
                self.pos += 1;
                match esc {
                    'd' => items.push(ClassItem::Digit(true)),
                    'D' => items.push(ClassItem::Digit(false)),
                    'w' => items.push(ClassItem::Word(true)),
                    'W' => items.push(ClassItem::Word(false)),
                    's' => items.push(ClassItem::Space(true)),
                    'S' => items.push(ClassItem::Space(false)),
                    't' => items.push(ClassItem::Single('\t')),
                    'n' => items.push(ClassItem::Single('\n')),
                    other => items.push(ClassItem::Single(self.fold_char(other))),
                }
            } else {
                let start_ch = self.fold_char(c);
                self.pos += 1;
                if self.pos + 1 < self.chars.len()
                    && self.chars[self.pos] == '-'
                    && self.chars[self.pos + 1] != ']'
                {
                    self.pos += 1; // skip '-'
                    let end_ch = self.fold_char(self.chars[self.pos]);
                    self.pos += 1;
                    let (lo, hi) = if start_ch <= end_ch {
                        (start_ch, end_ch)
                    } else {
                        (end_ch, start_ch)
                    };
                    items.push(ClassItem::Range(lo, hi));
                } else {
                    items.push(ClassItem::Single(start_ch));
                }
            }
        }
        Err("字符集缺少闭合括号 ']'".to_string())
    }
}
