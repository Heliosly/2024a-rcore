bitflags! {
    /// Mmap flags
    pub struct MmapFlags: u32 {
        /// Share changes
        const MAP_SHARED = 1;
        /// Changes are private
        const MAP_PRIVATE = 1 << 1;
        /// Interpret addr exactly
        const MAP_FIXED = 1 << 4;
        /// Don't use a file
        const MAP_ANONYMOUS = 1 << 5;
        /// Don't permit write
        const MAP_DENYWRITE = 1 << 11;
        /// Populate (prefault) page tables 
        const MAP_POPULATE = 1 << 13;
        /// Region grows down (like a stack)
        const MAP_STACK = 1 << 17;
        /// MAP_FIXED_NOREPLACE: Don't replace existing mapping
        const MAP_FIXED_NOREPLACE = 1 << 16;
    }
}

// For Mmap
bitflags! {
    /// Mmap permissions
    pub struct MmapProt: u32 {
        /// None
        const PROT_NONE = 0;
        /// Readable
        const PROT_READ = 1 << 0;
        /// Writable
        const PROT_WRITE = 1 << 1;
        /// Executable
        const PROT_EXEC = 1 << 2;
    }
}