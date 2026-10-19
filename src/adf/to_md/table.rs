//! Tables: 1×1 box cells (`::: cell`), GFM tables, and the plain flavour's best-effort tables.

use serde_json::Value;

use crate::adf::reader::Node;

use super::block::{Container, is_single_empty_paragraph, render_children};
use super::escape::{TextContext, TextEscape, code_span, escape_text};
use super::inline::{InlineKind, render_inline};
use super::{Ctx, Fail};

/// Column alignment of a GFM table (`alignment` mark on each cell's paragraph).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Align {
    Default,
    Center,
    End,
}

pub(super) fn render_table(node: &Node<'_>, cx: &mut Ctx) -> Result<String, Fail> {
    if let Some(cell) = box_cell(node) {
        return if cx.roundtrip {
            render_box(node, cell, cx)
        } else {
            plain_box(cell, cx)
        };
    }
    if cx.roundtrip {
        render_gfm(node, cx)
    } else {
        render_plain(node, cx)
    }
}

/// The cell of a 1×1 table holding one `tableCell` without spans.
fn box_cell<'n, 'a>(node: &'n Node<'a>) -> Option<&'n Node<'a>> {
    match node.content.as_slice() {
        [row] if row.ty == "tableRow" => match row.content.as_slice() {
            [cell] if cell.ty == "tableCell" && span_is_one(cell) => Some(cell),
            _ => None,
        },
        _ => None,
    }
}

fn span_is_one(cell: &Node<'_>) -> bool {
    ["colspan", "rowspan"].iter().all(|name| {
        cell.attr(name)
            .is_none_or(|v| v.is_null() || v.as_u64() == Some(1))
    })
}

fn drop_table_attrs(node: &Node<'_>, cx: &mut Ctx) {
    cx.drop_attrs(node, &[], &[("layout", "default")]);
    cx.drop_block_marks(node);
}

fn drop_cell_attrs(cell: &Node<'_>, cx: &mut Ctx) {
    cx.drop_attrs(cell, &["colspan", "rowspan"], &[]);
}

fn render_box(table: &Node<'_>, cell: &Node<'_>, cx: &mut Ctx) -> Result<String, Fail> {
    if cell.content.is_empty() {
        return Err(Fail::unrepresentable(
            &cell.pointer,
            "an empty table cell has no Markdown form",
        ));
    }
    drop_table_attrs(table, cx);
    if let Some(row) = table.content.first() {
        cx.drop_attrs(row, &[], &[]);
    }
    drop_cell_attrs(cell, cx);
    cx.enter(cell)?;
    let body = if is_single_empty_paragraph(&cell.content) {
        Ok(String::new())
    } else {
        render_children(&cell.content, Container::TableCell, cx).map(|blocks| {
            blocks
                .iter()
                .map(|b| b.md.as_str())
                .filter(|md| !md.is_empty())
                .collect::<Vec<_>>()
                .join("\n\n")
        })
    };
    cx.leave();
    let body = body?;
    // More colons than any line inside that starts with colons (code can hold such lines).
    let inner = body
        .split('\n')
        .map(|line| line.chars().take_while(|c| *c == ':').count())
        .max()
        .unwrap_or(0);
    let fence = ":".repeat((inner + 1).max(3));
    let mut md = format!("{fence} cell\n");
    if !body.is_empty() {
        md.push_str(&body);
        md.push('\n');
    }
    md.push_str(&fence);
    Ok(md)
}

fn plain_box(cell: &Node<'_>, cx: &mut Ctx) -> Result<String, Fail> {
    cx.enter(cell)?;
    let blocks = render_children(&cell.content, Container::TableCell, cx);
    cx.leave();
    Ok(blocks?
        .iter()
        .map(|b| b.md.as_str())
        .filter(|md| !md.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n"))
}

fn not_gfm(node: &Node<'_>, why: &str) -> Fail {
    Fail::unrepresentable(&node.pointer, &format!("a GFM table cannot hold it: {why}"))
}

fn render_gfm(table: &Node<'_>, cx: &mut Ctx) -> Result<String, Fail> {
    let rows = &table.content;
    let width = rows.first().map_or(0, |r| r.content.len());
    if width == 0 {
        return Err(not_gfm(table, "the table has no cells"));
    }
    for (r, row) in rows.iter().enumerate() {
        if row.ty != "tableRow" {
            return Err(not_gfm(row, "a table may only contain rows"));
        }
        if row.content.len() != width {
            return Err(not_gfm(row, "rows have different numbers of cells"));
        }
        let expected = if r == 0 { "tableHeader" } else { "tableCell" };
        for cell in &row.content {
            if cell.ty != expected {
                let why = if r == 0 {
                    "the first row must consist of header cells"
                } else {
                    "header cells are only possible in the first row"
                };
                return Err(not_gfm(cell, why));
            }
            if !span_is_one(cell) {
                return Err(not_gfm(cell, "cells span several rows or columns"));
            }
            match cell.content.as_slice() {
                [p] if p.ty == "paragraph" => {}
                _ => {
                    return Err(not_gfm(
                        cell,
                        "a cell holds something other than one paragraph",
                    ));
                }
            }
        }
    }
    let mut aligns: Vec<Option<Align>> = vec![None; width];
    for row in rows {
        for (cell, column) in row.content.iter().zip(aligns.iter_mut()) {
            let align = cell
                .content
                .first()
                .map_or(Ok(Align::Default), paragraph_align)
                .map_err(|why| not_gfm(cell, why))?;
            match column {
                Some(existing) if *existing != align => {
                    return Err(not_gfm(
                        cell,
                        "the cells of a column are aligned differently",
                    ));
                }
                Some(_) => {}
                None => *column = Some(align),
            }
        }
    }
    drop_table_attrs(table, cx);
    let mut lines = Vec::with_capacity(rows.len() + 1);
    for (r, row) in rows.iter().enumerate() {
        cx.drop_attrs(row, &[], &[]);
        let mut cells = Vec::with_capacity(width);
        for cell in &row.content {
            drop_cell_attrs(cell, cx);
            let paragraph = cell.content.first();
            if let Some(p) = paragraph {
                cx.drop_attrs(p, &[], &[]);
            }
            let content = match paragraph {
                Some(p) => render_inline(&p.content, InlineKind::TableCell, cx)?,
                None => String::new(),
            };
            cells.push(content);
        }
        lines.push(table_row(&cells));
        if r == 0 {
            let delims: Vec<String> = aligns
                .iter()
                .map(|a| delimiter(a.unwrap_or(Align::Default)).to_owned())
                .collect();
            lines.push(table_row(&delims));
        }
    }
    Ok(lines.join("\n"))
}

/// The alignment mark of a cell paragraph. Other block marks have no GFM form.
fn paragraph_align(paragraph: &Node<'_>) -> Result<Align, &'static str> {
    let mut align = Align::Default;
    for mark in &paragraph.marks {
        match (mark.ty, mark.attr_str("align")) {
            ("alignment", Some("center")) => align = Align::Center,
            ("alignment", Some("end")) => align = Align::End,
            ("alignment", _) => return Err("an alignment other than center or end"),
            _ => return Err("a cell paragraph carries a block mark other than alignment"),
        }
    }
    Ok(align)
}

fn delimiter(align: Align) -> &'static str {
    match align {
        Align::Default => "---",
        Align::Center => ":---:",
        Align::End => "---:",
    }
}

fn table_row(cells: &[String]) -> String {
    let mut line = String::from("|");
    for cell in cells {
        line.push(' ');
        line.push_str(cell);
        line.push_str(" |");
    }
    line
}

// ---------------------------------------------------------------------------------------------
// Plain flavour
// ---------------------------------------------------------------------------------------------

fn render_plain(table: &Node<'_>, cx: &mut Ctx) -> Result<String, Fail> {
    let mut grid: Vec<Vec<String>> = Vec::new();
    let mut aligns: Vec<Align> = Vec::new();
    for (r, row) in table
        .content
        .iter()
        .filter(|r| r.ty == "tableRow")
        .enumerate()
    {
        let mut cells = Vec::new();
        for cell in &row.content {
            cx.enter(cell)?;
            let text = plain_cell(cell, cx);
            cx.leave();
            if r == 0 {
                let align = cell
                    .content
                    .first()
                    .and_then(|p| paragraph_align(p).ok())
                    .unwrap_or(Align::Default);
                aligns.push(align);
            }
            cells.push(text?);
            let span = cell.attr("colspan").and_then(Value::as_u64).unwrap_or(1);
            for _ in 1..span.min(64) {
                cells.push(String::new());
                if r == 0 {
                    aligns.push(Align::Default);
                }
            }
        }
        grid.push(cells);
    }
    let width = grid.iter().map(Vec::len).max().unwrap_or(0);
    if width == 0 {
        return Ok(String::new());
    }
    aligns.resize(width, Align::Default);
    let mut lines = Vec::with_capacity(grid.len() + 1);
    for (r, mut cells) in grid.into_iter().enumerate() {
        cells.resize(width, String::new());
        lines.push(table_row(&cells));
        if r == 0 {
            let delims: Vec<String> = aligns.iter().map(|a| delimiter(*a).to_owned()).collect();
            lines.push(table_row(&delims));
        }
    }
    Ok(lines.join("\n"))
}

/// A cell's blocks as one line, joined with `<br>`.
fn plain_cell(cell: &Node<'_>, cx: &mut Ctx) -> Result<String, Fail> {
    let mut parts = Vec::new();
    for block in &cell.content {
        match block.ty {
            "paragraph" | "heading" => {
                let md = render_inline(&block.content, InlineKind::TableCell, cx)?;
                if !md.is_empty() {
                    parts.push(md);
                }
            }
            "bulletList" | "orderedList" | "taskList" => {
                for (i, item) in block.content.iter().enumerate() {
                    let marker = match block.ty {
                        "orderedList" => {
                            let start = block.attr("order").and_then(Value::as_u64).unwrap_or(1);
                            format!("{}.", start.saturating_add(u64::try_from(i).unwrap_or(0)))
                        }
                        "taskList" if item.attr_str("state") == Some("DONE") => "- [x]".to_owned(),
                        "taskList" => "- [ ]".to_owned(),
                        _ => "-".to_owned(),
                    };
                    parts.push(format!("{marker} {}", cell_text(&item.text_content())));
                }
            }
            "codeBlock" => {
                let text = block.text_content();
                for line in text.split('\n').filter(|l| !l.is_empty()) {
                    parts.push(code_span(line, true));
                }
            }
            _ => {
                let text = cell_text(&block.text_content());
                if !text.is_empty() {
                    parts.push(text);
                }
            }
        }
    }
    Ok(parts.join("<br>"))
}

fn cell_text(text: &str) -> String {
    let mut out = String::new();
    let lines: Vec<&str> = text.split('\n').collect();
    let last = lines.len().saturating_sub(1);
    for (i, line) in lines.iter().enumerate() {
        if i > 0 {
            out.push_str("<br>");
        }
        escape_text(
            line,
            TextEscape {
                context: TextContext::TableCell,
                roundtrip: false,
                line_start: i == 0,
                line_end: i == last,
                in_link: false,
            },
            &mut out,
        );
    }
    out
}
