/*
 * meli
 *
 * Copyright 2017-2018 Manos Pitsidianakis
 *
 * This file is part of meli.
 *
 * meli is free software: you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 *
 * meli is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public License
 * along with meli. If not, see <http://www.gnu.org/licenses/>.
 */

use super::*;
use crate::components::utilities::PageMovement;

const MAX_COLS: usize = 500;

/// A list of all mail (`Envelope`s) in a `Mailbox`. On `\n` it opens the `Envelope` content in a
/// `MailView`.
#[derive(Debug)]
pub struct ThreadListing {
    /// (x, y, z): x is accounts, y is mailboxes, z is index inside a mailbox.
    cursor_pos: (usize, MailboxHash, usize),
    new_cursor_pos: (usize, MailboxHash, usize),
    length: usize,
    sort: (SortField, SortOrder),
    subsort: (SortField, SortOrder),
    /// Cache current view.
    content: CellBuffer,
    color_cache: ColorCache,

    row_updates: SmallVec<[ThreadHash; 8]>,
    order: HashMap<EnvelopeHash, usize>,
    /// If we must redraw on next redraw event
    dirty: bool,
    /// If `self.view` is focused or not.
    unfocused: bool,
    initialised: bool,
    view: Option<MailView>,
    movement: Option<PageMovement>,
    id: ComponentId,
}

impl MailListingTrait for ThreadListing {
    fn row_updates(&mut self) -> &mut SmallVec<[ThreadHash; 8]> {
        &mut self.row_updates
    }

    fn get_focused_items(&self, _context: &Context) -> SmallVec<[ThreadHash; 8]> {
        SmallVec::new()
    }

    /// Fill the `self.content` `CellBuffer` with the contents of the account mailbox the user has
    /// chosen.
    fn refresh_mailbox(&mut self, context: &mut Context, _force: bool) {
        self.dirty = true;
        if !(self.cursor_pos.0 == self.new_cursor_pos.0
            && self.cursor_pos.1 == self.new_cursor_pos.1)
        {
            self.cursor_pos.2 = 0;
            self.new_cursor_pos.2 = 0;
        }
        self.cursor_pos.1 = self.new_cursor_pos.1;
        self.cursor_pos.0 = self.new_cursor_pos.0;

        self.color_cache = ColorCache {
            unseen: crate::conf::value(context, "mail.listing.plain.even_unseen"),
            highlighted: crate::conf::value(context, "mail.listing.plain.even_highlighted"),
            even: crate::conf::value(context, "mail.listing.plain.even"),
            odd: crate::conf::value(context, "mail.listing.plain.odd"),
            selected: crate::conf::value(context, "mail.listing.plain.even_selected"),
            attachment_flag: crate::conf::value(context, "mail.listing.attachment_flag"),
            thread_snooze_flag: crate::conf::value(context, "mail.listing.thread_snooze_flag"),
            ..self.color_cache
        };
        if !context.settings.terminal.use_color() {
            self.color_cache.highlighted.attrs |= Attr::REVERSE;
        }

        // Get mailbox as a reference.
        //
        match context.accounts[self.cursor_pos.0].load(self.cursor_pos.1) {
            Ok(_) => {}
            Err(_) => {
                let default_cell = {
                    let mut ret = Cell::with_char(' ');
                    ret.set_fg(self.color_cache.theme_default.fg)
                        .set_bg(self.color_cache.theme_default.bg)
                        .set_attrs(self.color_cache.theme_default.attrs);
                    ret
                };
                let message: String =
                    context.accounts[self.cursor_pos.0][&self.cursor_pos.1].status();
                self.content =
                    CellBuffer::new_with_context(message.len(), 1, default_cell, context);
                self.length = 0;
                write_string_to_grid(
                    message.as_str(),
                    &mut self.content,
                    self.color_cache.theme_default.fg,
                    self.color_cache.theme_default.bg,
                    self.color_cache.theme_default.attrs,
                    ((0, 0), (MAX_COLS - 1, 0)),
                    None,
                );
                return;
            }
        }
        let account = &context.accounts[self.cursor_pos.0];
        let threads = &account.collection.threads[&self.cursor_pos.1];
        self.length = 0;
        self.order.clear();
        let default_cell = {
            let mut ret = Cell::with_char(' ');
            ret.set_fg(self.color_cache.theme_default.fg)
                .set_bg(self.color_cache.theme_default.bg)
                .set_attrs(self.color_cache.theme_default.attrs);
            ret
        };
        if threads.len() == 0 {
            let message = format!("Mailbox `{}` is empty.", account[&self.cursor_pos.1].name());
            self.content = CellBuffer::new_with_context(message.len(), 1, default_cell, context);
            write_string_to_grid(
                &message,
                &mut self.content,
                self.color_cache.theme_default.fg,
                self.color_cache.theme_default.bg,
                self.color_cache.theme_default.attrs,
                ((0, 0), (message.len() - 1, 0)),
                None,
            );
            return;
        }
        self.content =
            CellBuffer::new_with_context(MAX_COLS, threads.len() + 1, default_cell, context);

        let mut indentations: Vec<bool> = Vec::with_capacity(6);
        let mut thread_idx = 0; // needed for alternate thread colors
                                /* Draw threaded view. */
        let mut roots = threads.roots();
        threads.group_inner_sort_by(&mut roots, self.sort, &account.collection.envelopes);
        let roots = roots
            .into_iter()
            .filter_map(|r| threads.groups[&r].root().map(|r| r.root))
            .collect::<_>();
        let mut iter = threads.threads_group_iter(roots).peekable();
        let thread_nodes: &HashMap<ThreadNodeHash, ThreadNode> = &threads.thread_nodes();
        /* This is just a desugared for loop so that we can use .peek() */
        let mut idx = 0;
        while let Some((indentation, thread_node_hash, has_sibling)) = iter.next() {
            let thread_node = &thread_nodes[&thread_node_hash];

            if indentation == 0 {
                thread_idx += 1;
            }
            if thread_node.has_message() {
                let envelope: EnvelopeRef =
                    account.collection.get_env(thread_node.message().unwrap());
                self.order.insert(envelope.hash(), idx);
                let fg_color = if !envelope.is_seen() {
                    Color::Byte(0)
                } else {
                    Color::Default
                };
                let bg_color = if !envelope.is_seen() {
                    Color::Byte(251)
                } else if thread_idx % 2 == 0 {
                    Color::Byte(236)
                } else {
                    Color::Default
                };
                let (x, _) = write_string_to_grid(
                    &ThreadListing::make_thread_entry(
                        &envelope,
                        idx,
                        indentation,
                        thread_node_hash,
                        threads,
                        &indentations,
                        has_sibling,
                    ),
                    &mut self.content,
                    fg_color,
                    bg_color,
                    Attr::DEFAULT,
                    ((0, idx), (MAX_COLS - 1, idx)),
                    None,
                );

                for x in x..MAX_COLS {
                    self.content[(x, idx)].set_ch(' ');
                    self.content[(x, idx)].set_bg(bg_color);
                }
                idx += 1;
            } else {
                continue;
            }

            match iter.peek() {
                Some((x, _, _)) if *x > indentation => {
                    if has_sibling {
                        indentations.push(true);
                    } else {
                        indentations.push(false);
                    }
                }
                Some((x, _, _)) if *x < indentation => {
                    for _ in 0..(indentation - *x) {
                        indentations.pop();
                    }
                }
                _ => {}
            }
        }
        self.length = self.order.len();
    }
}

impl ListingTrait for ThreadListing {
    fn coordinates(&self) -> (usize, MailboxHash) {
        (self.new_cursor_pos.0, self.new_cursor_pos.1)
    }
    fn set_coordinates(&mut self, coordinates: (usize, MailboxHash)) {
        self.new_cursor_pos = (coordinates.0, coordinates.1, 0);
        self.unfocused = false;
        self.view = None;
        self.order.clear();
        self.row_updates.clear();
        self.initialised = false;
    }

    fn draw_list(&mut self, grid: &mut CellBuffer, area: Area, context: &mut Context) {
        if self.cursor_pos.1 != self.new_cursor_pos.1 || self.cursor_pos.0 != self.new_cursor_pos.0
        {
            self.refresh_mailbox(context, false);
        }
        let upper_left = upper_left!(area);
        let bottom_right = bottom_right!(area);
        if self.length == 0 {
            clear_area(grid, area, self.color_cache.theme_default);
            copy_area(
                grid,
                &self.content,
                area,
                ((0, 0), pos_dec(self.content.size(), (1, 1))),
            );
            context.dirty_areas.push_back(area);
            return;
        }
        let rows = get_y(bottom_right) - get_y(upper_left) + 1;
        if rows == 0 {
            return;
        }
        if let Some(mvm) = self.movement.take() {
            match mvm {
                PageMovement::Up(amount) => {
                    self.new_cursor_pos.2 = self.new_cursor_pos.2.saturating_sub(amount);
                }
                PageMovement::PageUp(multiplier) => {
                    self.new_cursor_pos.2 = self.new_cursor_pos.2.saturating_sub(rows * multiplier);
                }
                PageMovement::Down(amount) => {
                    if self.new_cursor_pos.2 + amount + 1 < self.length {
                        self.new_cursor_pos.2 += amount;
                    } else {
                        self.new_cursor_pos.2 = self.length - 1;
                    }
                }
                PageMovement::PageDown(multiplier) => {
                    if self.new_cursor_pos.2 + rows * multiplier + 1 < self.length {
                        self.new_cursor_pos.2 += rows * multiplier;
                    } else if self.new_cursor_pos.2 + rows * multiplier > self.length {
                        self.new_cursor_pos.2 = self.length - 1;
                    } else {
                        self.new_cursor_pos.2 = (self.length / rows) * rows;
                    }
                }
                PageMovement::Right(_) | PageMovement::Left(_) => {}
                PageMovement::Home => {
                    self.new_cursor_pos.2 = 0;
                }
                PageMovement::End => {
                    self.new_cursor_pos.2 = self.length - 1;
                }
            }
        }

        let prev_page_no = (self.cursor_pos.2).wrapping_div(rows);
        let page_no = (self.new_cursor_pos.2).wrapping_div(rows);

        let top_idx = page_no * rows;
        if !self.initialised {
            self.initialised = false;
            copy_area(
                grid,
                &self.content,
                area,
                ((0, top_idx), (MAX_COLS - 1, self.length)),
            );
            self.highlight_line(
                grid,
                (
                    set_y(upper_left, get_y(upper_left) + (self.cursor_pos.2 % rows)),
                    set_y(bottom_right, get_y(upper_left) + (self.cursor_pos.2 % rows)),
                ),
                self.cursor_pos.2,
                context,
            );
            context.dirty_areas.push_back(area);
        }
        /* If cursor position has changed, remove the highlight from the previous position and
         * apply it in the new one. */
        if self.cursor_pos.2 != self.new_cursor_pos.2 && prev_page_no == page_no {
            let old_cursor_pos = self.cursor_pos;
            self.cursor_pos = self.new_cursor_pos;
            for idx in &[old_cursor_pos.2, self.new_cursor_pos.2] {
                if *idx >= self.length {
                    continue; //bounds check
                }
                let new_area = (
                    set_y(upper_left, get_y(upper_left) + (*idx % rows)),
                    set_y(bottom_right, get_y(upper_left) + (*idx % rows)),
                );
                self.highlight_line(grid, new_area, *idx, context);
                context.dirty_areas.push_back(new_area);
            }
            return;
        } else if self.cursor_pos != self.new_cursor_pos {
            self.cursor_pos = self.new_cursor_pos;
        }

        /* Page_no has changed, so draw new page */
        copy_area(
            grid,
            &self.content,
            area,
            ((0, top_idx), (MAX_COLS - 1, self.length)),
        );
        self.highlight_line(
            grid,
            (
                set_y(upper_left, get_y(upper_left) + (self.cursor_pos.2 % rows)),
                set_y(bottom_right, get_y(upper_left) + (self.cursor_pos.2 % rows)),
            ),
            self.cursor_pos.2,
            context,
        );
        context.dirty_areas.push_back(area);
    }

    fn highlight_line(&mut self, grid: &mut CellBuffer, area: Area, idx: usize, context: &Context) {
        if self.length == 0 {
            return;
        }

        let env_hash = self.get_env_under_cursor(idx, context);
        let envelope: EnvelopeRef = context.accounts[self.cursor_pos.0]
            .collection
            .get_env(env_hash);

        let fg_color = if !envelope.is_seen() {
            Color::Byte(0)
        } else {
            Color::Default
        };
        let bg_color = if self.cursor_pos.2 == idx {
            Color::Byte(246)
        } else if !envelope.is_seen() {
            Color::Byte(251)
        } else if idx % 2 == 0 {
            Color::Byte(236)
        } else {
            Color::Default
        };
        change_colors(grid, area, fg_color, bg_color);
    }

    fn set_movement(&mut self, mvm: PageMovement) {
        self.movement = Some(mvm);
        self.set_dirty(true);
    }
}

impl fmt::Display for ThreadListing {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "mail")
    }
}

impl ThreadListing {
    pub fn new(coordinates: (usize, MailboxHash)) -> Self {
        ThreadListing {
            cursor_pos: (0, 1, 0),
            new_cursor_pos: (coordinates.0, coordinates.1, 0),
            length: 0,
            sort: (Default::default(), Default::default()),
            subsort: (Default::default(), Default::default()),
            content: CellBuffer::new(0, 0, Cell::with_char(' ')),
            color_cache: ColorCache::default(),
            row_updates: SmallVec::new(),
            order: HashMap::default(),
            dirty: true,
            unfocused: false,
            view: None,
            initialised: false,
            movement: None,
            id: ComponentId::new_v4(),
        }
    }

    fn highlight_line_self(&mut self, idx: usize, context: &Context) {
        if self.length == 0 {
            return;
        }

        let env_hash = self.get_env_under_cursor(idx, context);
        let envelope: EnvelopeRef = context.accounts[self.cursor_pos.0]
            .collection
            .get_env(env_hash);

        let fg_color = if !envelope.is_seen() {
            Color::Byte(0)
        } else {
            Color::Default
        };
        let bg_color = if !envelope.is_seen() {
            Color::Byte(251)
        } else if idx % 2 == 0 {
            Color::Byte(236)
        } else {
            Color::Default
        };
        change_colors(
            &mut self.content,
            ((0, idx), (MAX_COLS - 1, idx)),
            fg_color,
            bg_color,
        );
    }

    fn make_thread_entry(
        envelope: &Envelope,
        idx: usize,
        indent: usize,
        node_idx: ThreadNodeHash,
        threads: &Threads,
        indentations: &[bool],
        has_sibling: bool,
        //op: Box<BackendOp>,
    ) -> String {
        let thread_node = &threads[&node_idx];
        let has_parent = thread_node.has_parent();
        let show_subject = thread_node.show_subject();

        let mut s = format!("{}{}{} ", idx, " ", ThreadListing::format_date(&envelope));
        for i in 0..indent {
            if indentations.len() > i && indentations[i] {
                s.push('│');
            } else if indentations.len() > i {
                s.push(' ');
            }
            if i > 0 {
                s.push(' ');
            }
        }
        if indent > 0 && (has_sibling || has_parent) {
            if has_sibling && has_parent {
                s.push('├');
            } else if has_sibling {
                s.push('┬');
            } else {
                s.push('└');
            }
            s.push('─');
            s.push('>');
        }

        s.push_str(if envelope.has_attachments() {
            "📎"
        } else {
            ""
        });
        if show_subject {
            s.push_str(&format!("{:.85}", envelope.subject()));
        }
        s
    }

    fn get_env_under_cursor(&self, cursor: usize, _context: &Context) -> EnvelopeHash {
        *self
            .order
            .iter()
            .find(|(_, &r)| r == cursor)
            .unwrap_or_else(|| {
                debug!("self.order empty ? cursor={} {:#?}", cursor, &self.order);
                panic!();
            })
            .0
    }

    fn format_date(envelope: &Envelope) -> String {
        let d = std::time::UNIX_EPOCH + std::time::Duration::from_secs(envelope.date());
        let now: std::time::Duration = std::time::SystemTime::now()
            .duration_since(d)
            .unwrap_or_else(|_| std::time::Duration::new(std::u64::MAX, 0));
        match now.as_secs() {
            n if n < 10 * 60 * 60 => format!("{} hours ago{}", n / (60 * 60), " ".repeat(8)),
            n if n < 24 * 60 * 60 => format!("{} hours ago{}", n / (60 * 60), " ".repeat(7)),
            n if n < 4 * 24 * 60 * 60 => {
                format!("{} days ago{}", n / (24 * 60 * 60), " ".repeat(9))
            }
            _ => melib::datetime::timestamp_to_string(envelope.datetime(), None),
        }
    }
}

impl Component for ThreadListing {
    fn draw(&mut self, grid: &mut CellBuffer, area: Area, context: &mut Context) {
        if !self.unfocused {
            if !self.is_dirty() {
                return;
            }
            self.dirty = false;
            /* Draw the entire list */
            self.draw_list(grid, area, context);
        } else {
            self.cursor_pos = self.new_cursor_pos;
            let upper_left = upper_left!(area);
            let bottom_right = bottom_right!(area);
            if self.length == 0 && self.dirty {
                clear_area(grid, area, self.color_cache.theme_default);
                context.dirty_areas.push_back(area);
                return;
            }

            /* Render the mail body in a pager, basically copy what HSplit does */
            let total_rows = get_y(bottom_right) - get_y(upper_left);
            let pager_ratio = context.runtime_settings.pager.pager_ratio;
            let bottom_entity_rows = (pager_ratio * total_rows) / 100;

            if bottom_entity_rows > total_rows {
                clear_area(grid, area, self.color_cache.theme_default);
                context.dirty_areas.push_back(area);
                return;
            }

            let idx = self.cursor_pos.2;

            /* Mark message as read */
            let must_highlight = {
                if self.length == 0 {
                    false
                } else {
                    let account = &context.accounts[self.cursor_pos.0];
                    let envelope: EnvelopeRef = account
                        .collection
                        .get_env(self.get_env_under_cursor(idx, context));
                    envelope.is_seen()
                }
            };

            if must_highlight {
                self.highlight_line_self(idx, context);
            }

            let mid = get_y(upper_left) + total_rows - bottom_entity_rows;
            self.draw_list(
                grid,
                (
                    upper_left,
                    (get_x(bottom_right), get_y(upper_left) + mid - 1),
                ),
                context,
            );
            if self.length == 0 {
                self.dirty = false;
                return;
            }
            {
                /* TODO: Move the box drawing business in separate functions */
                if get_x(upper_left) > 0 && grid[(get_x(upper_left) - 1, mid)].ch() == VERT_BOUNDARY
                {
                    grid[(get_x(upper_left) - 1, mid)].set_ch(LIGHT_VERTICAL_AND_RIGHT);
                }

                for i in get_x(upper_left)..=get_x(bottom_right) {
                    grid[(i, mid)].set_ch(HORZ_BOUNDARY);
                }
                context
                    .dirty_areas
                    .push_back((set_y(upper_left, mid), set_y(bottom_right, mid)));
            }
            // TODO: Make headers view configurable

            if !self.dirty {
                if let Some(v) = self.view.as_mut() {
                    v.draw(grid, (set_y(upper_left, mid + 1), bottom_right), context);
                }
                return;
            }

            let coordinates = (
                self.cursor_pos.0,
                self.cursor_pos.1,
                self.get_env_under_cursor(self.cursor_pos.2, context),
            );

            if let Some(ref mut v) = self.view {
                v.update(coordinates);
            } else {
                self.view = Some(MailView::new(coordinates, None, None, context));
            }

            self.view.as_mut().unwrap().draw(
                grid,
                (set_y(upper_left, mid + 1), bottom_right),
                context,
            );

            self.dirty = false;
        }
    }
    fn process_event(&mut self, event: &mut UIEvent, context: &mut Context) -> bool {
        if let Some(ref mut v) = self.view {
            if v.process_event(event, context) {
                return true;
            }
        }
        match *event {
            UIEvent::Input(Key::Char('\n')) if !self.unfocused => {
                self.unfocused = true;
                self.dirty = true;
                return true;
            }
            UIEvent::Input(Key::Char('i')) if self.unfocused => {
                self.unfocused = false;
                self.dirty = true;
                self.view = None;
                return true;
            }
            UIEvent::MailboxUpdate((ref idxa, ref idxf))
                if (*idxa, *idxf) == (self.new_cursor_pos.0, self.cursor_pos.1) =>
            {
                self.refresh_mailbox(context, false);
                self.set_dirty(true);
            }
            UIEvent::StartupCheck(ref f) if *f == self.cursor_pos.1 => {
                self.refresh_mailbox(context, false);
                self.set_dirty(true);
            }
            UIEvent::ChangeMode(UIMode::Normal) => {
                self.dirty = true;
            }
            UIEvent::Resize => {
                self.dirty = true;
            }
            UIEvent::Action(ref action) => match action {
                Action::SubSort(field, order) => {
                    debug!("SubSort {:?} , {:?}", field, order);
                    self.subsort = (*field, *order);
                    self.dirty = true;
                    self.refresh_mailbox(context, false);
                    return true;
                }
                Action::Sort(field, order) => {
                    debug!("Sort {:?} , {:?}", field, order);
                    self.sort = (*field, *order);
                    self.dirty = true;
                    self.refresh_mailbox(context, false);
                    return true;
                }
                _ => {}
            },
            _ => {}
        }
        false
    }
    fn is_dirty(&self) -> bool {
        self.dirty || self.view.as_ref().map(|p| p.is_dirty()).unwrap_or(false)
    }
    fn set_dirty(&mut self, value: bool) {
        if let Some(p) = self.view.as_mut() {
            p.set_dirty(value);
        };
        self.dirty = value;
    }
    fn get_shortcuts(&self, context: &Context) -> ShortcutMaps {
        self.view
            .as_ref()
            .map(|p| p.get_shortcuts(context))
            .unwrap_or_default()
    }

    fn id(&self) -> ComponentId {
        self.id
    }
    fn set_id(&mut self, id: ComponentId) {
        self.id = id;
    }
}
