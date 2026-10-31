use super::values::{span, tint};
use super::{CellRange, El, Importer, RowHeightMode, StoryId, Table, names, swatch};

/// Preserve the legacy uniform field only when all four sides explicitly specify every
/// property. Partial records must stay sparse so they can inherit from a parent style.
pub(super) fn uniform_cell_stroke(edges: &[designcraft_doc::CellStrokeAttrs; 4]) -> Option<designcraft_doc::CellStroke> {
    let first = edges.first()?;
    if edges.iter().any(|edge| edge != first) {
        return None;
    }
    let mut stroke = designcraft_doc::CellStroke {
        weight: first.weight?,
        color: first.color.clone()?,
        tint: first.tint?,
        kind: first.kind.clone()?,
        ..Default::default()
    };
    first.apply_to(&mut stroke);
    Some(stroke)
}

impl<'r> Importer<'r> {
    /// `<Table>`: rows, columns and cells (cell `Name` is `column:row`).
    pub(super) fn table(&mut self, e: &El) -> Table {
        let id = self.alloc();
        let rows: Vec<&El> = e.find_all("Row").collect();
        let cols: Vec<&El> = e.find_all("Column").collect();
        let nr = rows.len().max(1);
        let nc = cols.len().max(1);
        let mut t = Table::new(id, nr, nc, 0, 0, 0.0);
        t.options.direction = if e.get("TableDirection") == Some("RightToLeftDirection") {
            designcraft_doc::TextDirection::RightToLeft
        } else {
            designcraft_doc::TextDirection::LeftToRight
        };
        for (r, re) in rows.iter().enumerate() {
            let auto = re.boolean("AutoGrow").unwrap_or(true);
            t.rows[r].mode = if auto { RowHeightMode::AtLeast } else { RowHeightMode::Exactly };
            t.rows[r].height = if auto { re.num("MinimumHeight").unwrap_or(3.0) } else { re.num("SingleRowHeight").unwrap_or(12.0) };
        }
        for (c, ce) in cols.iter().enumerate() {
            t.columns[c].width = ce.num("SingleColumnWidth").unwrap_or(72.0).max(3.0);
        }
        let h = e.num("HeaderRowCount").unwrap_or(0.0) as usize;
        let f = e.num("FooterRowCount").unwrap_or(0.0) as usize;
        if h.saturating_add(f) < nr {
            t.set_header_footer(h, f);
        }
        let style_name = e.get("AppliedTableStyle").map(|r| self.table_style_ref(r)).unwrap_or_else(|| designcraft_doc::BASIC_TABLE.into());
        let style = self.styles.resolve_table_style(&style_name);
        style.apply_to(&mut t, &self.styles.cell);
        // IDML distinguishes an explicit AppliedCellStyle from the effective region style.
        for cell in &mut t.cells {
            cell.style.clear();
        }
        // Table-local options override the named table style. Cell styles are already
        // materialized independently, so their edges retain their higher precedence.
        let mut border_unset = style.border.is_none() && style.borders.iter().all(|b| b.is_empty());
        let mut borders = std::array::from_fn::<_, 4, _>(|i| t.options.border_for(i).clone());
        for (side, border) in ["TopBorder", "LeftBorder", "BottomBorder", "RightBorder"].iter().zip(&mut borders) {
            let attrs = self.cell_stroke_attrs(e, side);
            border_unset &= attrs.is_empty();
            attrs.apply_to(border);
        }
        t.options.border = borders[0].clone();
        t.options.borders = borders.map(Some);
        t.options.space_before = e.num("SpaceBefore").unwrap_or(t.options.space_before);
        t.options.space_after = e.num("SpaceAfter").unwrap_or(t.options.space_after);
        t.options.repeat_header = e.get("HeaderBehavior") != Some("RepeatOnce");
        t.options.repeat_footer = e.get("FooterBehavior") != Some("RepeatOnce");
        // A local count may enable a pattern disabled by its parent style. Merge before
        // materialization so the parent's colors/tints are still available in that case.
        let mut row_fills = style.row_fills.clone();
        row_fills.merge(&self.alt_fill_attrs(e, "Row"));
        row_fills.apply_to(&mut t.options.alt_rows);
        let mut column_fills = style.column_fills.clone();
        column_fills.merge(&self.alt_fill_attrs(e, "Column"));
        column_fills.apply_to(&mut t.options.alt_cols);
        let mut row_strokes = style.row_strokes.clone();
        row_strokes.merge(&self.alt_stroke_attrs(e, "Row"));
        row_strokes.apply_to(&mut t.options.row_strokes);
        let mut column_strokes = style.column_strokes.clone();
        column_strokes.merge(&self.alt_stroke_attrs(e, "Column"));
        column_strokes.apply_to(&mut t.options.column_strokes);
        let mut regions = Vec::new();
        for ce in e.find_all("Cell") {
            let Some((c, r)) =
                ce.get("Name").and_then(|n| n.split_once(':')).and_then(|(a, b)| Some((a.parse::<usize>().ok()?, b.parse::<usize>().ok()?)))
            else {
                continue;
            };
            if r >= nr || c >= nc {
                continue;
            }
            let rs = (span(ce, "RowSpan").max(1.0) as usize).min(nr - r);
            let cs = (span(ce, "ColumnSpan").max(1.0) as usize).min(nc - c);
            let mut cell = t.cell(r, c).cloned().unwrap_or_default();
            let cell_style = ce.get("AppliedCellStyle").map(|r| self.cell_style_ref(r)).unwrap_or_else(|| designcraft_doc::NO_CELL_STYLE.into());
            if cell_style != designcraft_doc::NO_CELL_STYLE {
                designcraft_doc::resolve_cell_style(&self.styles.cell, &cell_style).apply_to(&mut cell);
            }
            cell.style = if cell_style == designcraft_doc::NO_CELL_STYLE { String::new() } else { cell_style };
            let default_para = cell.text.paras.first().cloned().unwrap_or_default();
            cell.text = self.story_with_para(StoryId(0), ce, default_para);
            self.read_cell_attrs(&mut cell, ce, border_unset);
            if let Some(slot) = t.cell_mut(r, c) {
                *slot = cell;
            }
            if rs > 1 || cs > 1 {
                regions.push(CellRange { r0: r, c0: c, r1: (r + rs - 1).min(nr - 1), c1: (c + cs - 1).min(nc - 1) });
            }
        }
        for rg in regions {
            let _ = t.merge(rg);
        }
        t
    }

    /// A cell's local fill, insets, alignment, rotation and edge strokes.
    fn read_cell_attrs(&mut self, cell: &mut designcraft_doc::Cell, ce: &El, border_unset: bool) {
        if let Some(fc) = ce.get("FillColor") {
            cell.fill = self.swatch_ref(fc);
        }
        if let Some(tint) = tint(ce.num("FillTint")) {
            cell.fill_tint = tint;
        }
        for (i, k) in ["TopInset", "LeftInset", "BottomInset", "RightInset"].iter().enumerate() {
            if let Some(v) = ce.num(k) {
                cell.insets[i] = v.max(0.0);
            }
        }
        if let Some(vj) = ce.get("VerticalJustification") {
            cell.vj = names::vj_in(vj);
        }
        if let Some(fb) = ce.get("FirstBaselineOffset") {
            cell.first_baseline = names::first_baseline_in(fb);
        }
        if let Some(min) = ce.num("MinimumFirstBaselineOffset").filter(|v| v.is_finite()) {
            cell.first_baseline_min = min.clamp(0.0, 8640.0);
        }
        cell.rotation = ce.num("RotationAngle").unwrap_or(cell.rotation);
        if cell.rotation != 0.0 {
            self.rotated_cells += 1;
        }
        for (i, side) in ["TopEdge", "LeftEdge", "BottomEdge", "RightEdge"].iter().enumerate() {
            let attrs = self.cell_stroke_attrs(ce, side);
            attrs.apply_to(&mut cell.strokes[i]);
            cell.stroke_defined[i] |= !attrs.is_empty();
            let priority = ce.prop(&format!("{side}StrokePriority")).and_then(|v| v.trim().parse::<i32>().ok()).unwrap_or(0);
            cell.stroke_priorities[i] = priority;
            // Preserve style-defined edges and #102's positive-priority local overrides.
            cell.border_overrides[i] |= priority > 0;
            // No table or table style border is given anywhere, so the built-in 1 pt default must not
            // draw over an outer edge the cell explicitly hides (weight 0 or the None swatch), as InDesign draws nothing.
            let hidden = attrs.weight == Some(0.0) || attrs.color.as_deref() == Some(swatch::NONE);
            cell.border_overrides[i] |= border_unset && hidden;
        }
    }
}
