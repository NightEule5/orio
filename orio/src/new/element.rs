// SPDX-License-Identifier: Apache-2.0

pub trait Element: bytemuck::Pod + Unpin { }

impl<T: bytemuck::Pod + Unpin> Element for T { }
