//! Layout primitives: axis, direction, sidebar slot enums.
//!
//! The layout *tree* lives in `crate::tree` as a `Box<dyn Pane>`. This
//! module is just the small enums consumers (split, focus, sidebar
//! toggling) use to express axis-or-direction.

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Axis { Horizontal, Vertical }

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum SidebarSlot { Left, Right }

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Direction { Left, Down, Up, Right }
