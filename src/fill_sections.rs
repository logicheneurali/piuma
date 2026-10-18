//! The layout widget that stretches the section rows over the bar's
//! full length (the `fill` rows), owning their MouseArea wiring.

use crate::Message;
use iced::advanced::layout::{Limits, Node};
use iced::advanced::widget::{Operation, Tree, Widget};
use iced::advanced::{mouse, overlay, renderer, Clipboard, Layout, Shell};
use iced::{Element, Event, Length, Rectangle, Size, Vector};

// ── fill sections widget ───────────────────────────────────────────

/// Sections container used by `align: "fill"`: it fills the whole main axis
/// of the panel and distributes the sections along it, so the panel always
/// spans 100% of the available screen space (width for a horizontal panel,
/// height for a vertical one). Sections keep their intrinsic size; only the
/// gaps between them move.
///
/// The offsets are recomputed on every layout pass from the measured
/// section sizes:
///
/// * **exactly 3 sections** and enough room for both side gaps → the middle
///   section is centred on the panel: `x1 = avail/2 - w1/2`, which implies
///   `gap_left = avail/2 - w0 - w1/2` and `gap_right = avail/2 - w2 - w1/2`;
/// * any other section count, or not enough room (a gap would be negative)
///   → fallback to the standard distribution: equal spacers between the
///   sections (space-between: the `n-1` gaps share the free space, clamped
///   to 0 when the content overflows the panel).
pub(crate) struct FillSections<'a> {
    /// Main axis of the panel: `true` for a vertical panel (y axis).
    pub(crate) vertical: bool,
    pub(crate) children: Vec<Element<'a, Message>>,
}

impl<'a> FillSections<'a> {
    pub(crate) fn new(vertical: bool, children: Vec<Element<'a, Message>>) -> Self {
        Self { vertical, children }
    }

    /// Main-axis offset of each section for the given measured sizes; the
    /// two distribution rules are documented on [`FillSections`].
    fn distribute(mains: &[f32], avail: f32) -> Vec<f32> {
        let n = mains.len();

        // 3 sections: centre the middle one if both side gaps stay >= 0
        // (which also guarantees the sections fit the panel).
        if n == 3 {
            let gap_left = avail / 2.0 - mains[0] - mains[1] / 2.0;
            let gap_right = avail / 2.0 - mains[2] - mains[1] / 2.0;
            if gap_left >= 0.0 && gap_right >= 0.0 {
                return vec![0.0, avail / 2.0 - mains[1] / 2.0, avail - mains[2]];
            }
        }

        // Standard: equal spacers, first section flush at the start edge,
        // last one at the end edge (0 gap when the content overflows).
        let content: f32 = mains.iter().sum();
        let gap = if n > 1 {
            (avail - content).max(0.0) / (n as f32 - 1.0)
        } else {
            0.0
        };
        let mut x = 0.0;
        mains
            .iter()
            .map(|&m| {
                let at = x;
                x += m + gap;
                at
            })
            .collect()
    }
}

impl Widget<Message, iced::Theme, iced::Renderer> for FillSections<'_> {
    fn children(&self) -> Vec<Tree> {
        self.children.iter().map(Tree::new).collect()
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(&self.children);
    }

    fn size(&self) -> Size<Length> {
        Size {
            width: Length::Fill,
            height: Length::Fill,
        }
    }

    /// Measures every section unbounded on the main axis (its intrinsic size
    /// decides the positions) and capped to the panel thickness on the cross
    /// axis (`Fill` children resolve to `max_cross`), then places each node
    /// at the offset given by [`FillSections::distribute`].
    fn layout(&mut self, tree: &mut Tree, renderer: &iced::Renderer, limits: &Limits) -> Node {
        // Main axis = width (horizontal panel) / height (vertical); the
        // cross axis carries the thickness and every section spans it fully.
        let (max_main, max_cross) = if self.vertical {
            (limits.max().height, limits.max().width)
        } else {
            (limits.max().width, limits.max().height)
        };

        let mut nodes = Vec::with_capacity(self.children.len());
        let mut mains: Vec<f32> = Vec::with_capacity(self.children.len());

        for (child, subtree) in self.children.iter_mut().zip(tree.children.iter_mut()) {
            let child_limits = Limits::new(
                Size::ZERO,
                if self.vertical {
                    Size::new(max_cross, f32::INFINITY)
                } else {
                    Size::new(f32::INFINITY, max_cross)
                },
            );
            let node = child
                .as_widget_mut()
                .layout(subtree, renderer, &child_limits);
            mains.push(if self.vertical {
                node.size().height
            } else {
                node.size().width
            });
            nodes.push(node);
        }

        let positions = Self::distribute(&mains, max_main);
        for (node, &at) in nodes.iter_mut().zip(&positions) {
            node.move_to_mut(if self.vertical { (0.0, at) } else { (at, 0.0) });
        }

        // Fill on both axes: the panel node spans the whole surface.
        let size = limits.resolve(Length::Fill, Length::Fill, Size::ZERO);
        Node::with_children(size, nodes)
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn Operation,
    ) {
        operation.container(None, layout.bounds());
        operation.traverse(&mut |operation| {
            self.children
                .iter_mut()
                .zip(&mut tree.children)
                .zip(layout.children())
                .for_each(|((child, state), layout)| {
                    child
                        .as_widget_mut()
                        .operate(state, layout, renderer, operation);
                });
        });
    }

    /// Forwards the events to the sections (hover/click on the sensors).
    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        for ((child, tree), layout) in self
            .children
            .iter_mut()
            .zip(&mut tree.children)
            .zip(layout.children())
        {
            child.as_widget_mut().update(
                tree, event, layout, cursor, renderer, clipboard, shell, viewport,
            );
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        self.children
            .iter()
            .zip(&tree.children)
            .zip(layout.children())
            .map(|((child, tree), layout)| {
                child
                    .as_widget()
                    .mouse_interaction(tree, layout, cursor, viewport, renderer)
            })
            .max()
            .unwrap_or_default()
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        theme: &iced::Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        for ((child, tree), layout) in self
            .children
            .iter()
            .zip(&tree.children)
            .zip(layout.children())
            .filter(|(_, layout)| layout.bounds().intersects(viewport))
        {
            child
                .as_widget()
                .draw(tree, renderer, theme, style, layout, cursor, viewport);
        }
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, iced::Theme, iced::Renderer>> {
        overlay::from_children(
            &mut self.children,
            tree,
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}

impl<'a> From<FillSections<'a>> for Element<'a, Message> {
    fn from(fill: FillSections<'a>) -> Self {
        Element::new(fill)
    }
}
