//! All meshes live in one big vertex buffer and one big index buffer ("arena"), so drawing a
//! mesh never rebinds buffers: each draw is just `draw_indexed` with a base vertex and first
//! index. Ranges are handed out by a small first-fit allocator; the buffers double in size when
//! they run out of room.

use std::ops::Range;

use crate::gfx::{MeshData, Vertex};

/// First-fit allocator of element ranges inside a growable buffer.
#[derive(Debug, Clone)]
pub(crate) struct RangeAllocator {
    capacity: u32,
    /// Free ranges, sorted by start and never adjacent (adjacent ranges are merged).
    free: Vec<Range<u32>>,
}

impl RangeAllocator {
    pub fn new(capacity: u32) -> Self {
        let mut free = Vec::new();
        if capacity > 0 {
            free.push(0..capacity);
        }
        Self { capacity, free }
    }

    pub fn capacity(&self) -> u32 {
        self.capacity
    }

    /// Allocates `len` elements. Zero-length allocations always succeed and occupy nothing.
    pub fn alloc(&mut self, len: u32) -> Option<Range<u32>> {
        if len == 0 {
            return Some(0..0);
        }
        let slot = self.free.iter().position(|r| r.end - r.start >= len)?;
        let range = &mut self.free[slot];
        let start = range.start;
        range.start += len;
        if range.start == range.end {
            self.free.remove(slot);
        }
        Some(start..start + len)
    }

    pub fn free(&mut self, range: Range<u32>) {
        if range.start >= range.end {
            return;
        }
        let at = self.free.partition_point(|r| r.start < range.start);
        self.free.insert(at, range);
        // Merge with the next range, then with the previous one.
        if at + 1 < self.free.len() && self.free[at].end == self.free[at + 1].start {
            self.free[at].end = self.free[at + 1].end;
            self.free.remove(at + 1);
        }
        if at > 0 && self.free[at - 1].end == self.free[at].start {
            self.free[at - 1].end = self.free[at].end;
            self.free.remove(at);
        }
    }

    /// Extends the managed space to `new_capacity` (must not shrink).
    pub fn grow(&mut self, new_capacity: u32) {
        if new_capacity <= self.capacity {
            return;
        }
        let old = self.capacity;
        self.capacity = new_capacity;
        self.free(old..new_capacity);
    }

    #[cfg(test)]
    fn free_total(&self) -> u32 {
        self.free.iter().map(|r| r.end - r.start).sum()
    }
}

/// Where one mesh lives inside the arena.
#[derive(Debug, Clone, Default)]
pub(crate) struct MeshAlloc {
    pub vertices: Range<u32>,
    pub indices: Range<u32>,
}

pub(crate) struct MeshArena {
    pub vertex_buffer: wgpu::Buffer,
    pub index_buffer: wgpu::Buffer,
    vertices: RangeAllocator,
    indices: RangeAllocator,
}

const VERTEX_SIZE: u64 = std::mem::size_of::<Vertex>() as u64;
const INDEX_SIZE: u64 = 4;
const INITIAL_VERTICES: u32 = 1 << 16;
const INITIAL_INDICES: u32 = 3 << 16;

impl MeshArena {
    pub fn new(device: &wgpu::Device) -> Self {
        Self {
            vertex_buffer: create_buffer(
                device,
                "mesh vertices",
                INITIAL_VERTICES as u64 * VERTEX_SIZE,
                wgpu::BufferUsages::VERTEX,
            ),
            index_buffer: create_buffer(
                device,
                "mesh indices",
                INITIAL_INDICES as u64 * INDEX_SIZE,
                wgpu::BufferUsages::INDEX,
            ),
            vertices: RangeAllocator::new(INITIAL_VERTICES),
            indices: RangeAllocator::new(INITIAL_INDICES),
        }
    }

    /// Uploads `data`, reusing `previous` when the new data fits into it.
    pub fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        data: &MeshData,
        previous: Option<MeshAlloc>,
    ) -> MeshAlloc {
        let (nv, ni) = (data.vertices.len() as u32, data.indices.len() as u32);
        let alloc = match previous {
            Some(prev) if prev.vertices.len() as u32 >= nv && prev.indices.len() as u32 >= ni => {
                prev
            }
            prev => {
                if let Some(prev) = prev {
                    self.free(prev);
                }
                MeshAlloc {
                    vertices: self.alloc_vertices(device, queue, nv),
                    indices: self.alloc_indices(device, queue, ni),
                }
            }
        };
        if nv > 0 {
            queue.write_buffer(
                &self.vertex_buffer,
                alloc.vertices.start as u64 * VERTEX_SIZE,
                bytemuck::cast_slice(&data.vertices),
            );
        }
        if ni > 0 {
            queue.write_buffer(
                &self.index_buffer,
                alloc.indices.start as u64 * INDEX_SIZE,
                bytemuck::cast_slice(&data.indices),
            );
        }
        alloc
    }

    pub fn free(&mut self, alloc: MeshAlloc) {
        self.vertices.free(alloc.vertices);
        self.indices.free(alloc.indices);
    }

    fn alloc_vertices(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        len: u32,
    ) -> Range<u32> {
        if let Some(range) = self.vertices.alloc(len) {
            return range;
        }
        let new_cap = grown_capacity(self.vertices.capacity(), len);
        self.vertex_buffer = grow_buffer(
            device,
            queue,
            &self.vertex_buffer,
            new_cap as u64 * VERTEX_SIZE,
            "mesh vertices",
            wgpu::BufferUsages::VERTEX,
        );
        self.vertices.grow(new_cap);
        self.vertices.alloc(len).unwrap_or(0..0)
    }

    fn alloc_indices(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        len: u32,
    ) -> Range<u32> {
        if let Some(range) = self.indices.alloc(len) {
            return range;
        }
        let new_cap = grown_capacity(self.indices.capacity(), len);
        self.index_buffer = grow_buffer(
            device,
            queue,
            &self.index_buffer,
            new_cap as u64 * INDEX_SIZE,
            "mesh indices",
            wgpu::BufferUsages::INDEX,
        );
        self.indices.grow(new_cap);
        self.indices.alloc(len).unwrap_or(0..0)
    }
}

/// Capacity after growing so that a `len` allocation surely fits (even if the tail is in use).
fn grown_capacity(capacity: u32, len: u32) -> u32 {
    capacity.saturating_mul(2).max(capacity.saturating_add(len))
}

fn create_buffer(
    device: &wgpu::Device,
    label: &str,
    size: u64,
    usage: wgpu::BufferUsages,
) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size,
        usage: usage | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    })
}

/// Creates a bigger buffer and copies the old contents into it (submitted immediately, so
/// later `write_buffer` calls for the new space are ordered after the copy).
fn grow_buffer(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    old: &wgpu::Buffer,
    new_size: u64,
    label: &str,
    usage: wgpu::BufferUsages,
) -> wgpu::Buffer {
    log::debug!("growing {label} buffer to {} KiB", new_size / 1024);
    let new = create_buffer(device, label, new_size, usage);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("arena grow"),
    });
    encoder.copy_buffer_to_buffer(old, 0, &new, 0, old.size());
    queue.submit([encoder.finish()]);
    new
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alloc_free_merges_ranges() {
        let mut a = RangeAllocator::new(100);
        let r1 = a.alloc(10).unwrap();
        let r2 = a.alloc(20).unwrap();
        let r3 = a.alloc(30).unwrap();
        assert_eq!(
            (r1.clone(), r2.clone(), r3.clone()),
            (0..10, 10..30, 30..60)
        );
        a.free(r2);
        // Fits in the hole left by r2 (first fit).
        assert_eq!(a.alloc(15).unwrap(), 10..25);
        a.free(10..25);
        a.free(r1);
        a.free(r3);
        assert_eq!((a.free.len(), a.free[0].clone()), (1, 0..100));
        assert_eq!(a.free_total(), 100);
    }

    #[test]
    fn grow_extends_tail() {
        let mut a = RangeAllocator::new(10);
        assert_eq!(a.alloc(8).unwrap(), 0..8);
        assert!(a.alloc(5).is_none());
        a.grow(20);
        assert_eq!(a.alloc(5).unwrap(), 8..13);
        assert_eq!(a.free_total(), 7);
        assert_eq!(a.alloc(0).unwrap(), 0..0);
    }

    #[test]
    fn grown_capacity_always_fits() {
        assert!(grown_capacity(16, 100) >= 116);
        assert_eq!(grown_capacity(64, 10), 128);
    }
}
