use core::num::{NonZeroU32, NonZeroU64};
use std::io::{self, Read, Seek, SeekFrom, Write};

use bytemuck::{Pod, Zeroable};

use crate::traits::{Search, StreamId};

bitflags::bitflags! {
    #[derive(Default,Zeroable,Pod)]
    #[repr(transparent)]
    pub struct PhantomFSObjectFlags : u32{

    }
}

bitflags::bitflags! {
    #[derive(Default,Zeroable,Pod)]
    #[repr(transparent)]
    pub struct PhantomFSStreamFlags : u64 {
        const REQUIRED       = 0x0000000000000001;
        const WRITE_REQUIRED = 0x0000000000000002;
        const ENUM_REQUIRED  = 0x0000000000000004;
    }
}

fake_enum::fake_enum! {
    #[repr(u16)]
    #[derive(Hash,Zeroable,Pod)]
    pub enum struct PhantomFSObjectType{
        Regular = 0,
        Directory = 1,
        Symlink = 2,
        Fifo = 3,
        Socket = 4,
        BlockDeivce = 5,
        CharDevice = 6,
        CustomType = 65535
    }
}

#[repr(C, align(64))]
#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq, Zeroable, Pod)]
pub struct PhantomFSObject {
    strong_ref: u32,
    weak_ref: Option<NonZeroU32>,
    streams_size: u64,
    streams_ref: u128,
    streams_indirection: u8,
    reserved33: [u8; 5],
    ty: PhantomFSObjectType,
    flags: PhantomFSObjectFlags,
    reserved44: [u8; 20],
}

#[repr(C, align(128))]
#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq, Zeroable, Pod)]
pub struct StreamListing {
    name: [u8; 32],
    name_ref: Option<NonZeroU64>,
    flags: PhantomFSStreamFlags,
    size: u64,
    reserved: [u64; 3],
    inline_data: [u8; 48],
}

#[repr(C, align(64))]
#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq, Zeroable, Pod)]
pub struct DirectoryElement {
    objidx: Option<NonZeroU64>,
    name_index: Option<NonZeroU64>,
    flags: u64,
    name: [u8; 40],
}

#[repr(C, align(16))]
#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq, Zeroable, Pod)]
pub struct DeviceId {
    id_hi: u64,
    id_lo: u64,
}

#[repr(C, align(8))]
#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq, Zeroable, Pod)]
pub struct LegacyDeviceNumber {
    major: u32,
    minor: u32,
}

#[repr(C, align(64))]
#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq, Zeroable, Pod)]
pub struct SecurityDescriptorRow {
    principal: u128,
    stream_id: Option<NonZeroU64>,
    flags_and_mode: u64,
    permission_name_ref: Option<NonZeroU64>,
    permission_name: [u8; 24],
}

#[repr(C, align(16))]
#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq, Zeroable, Pod)]
pub struct LegacySecurityDescriptor {
    sd_uid: u32,
    sd_gid: u32,
    sd_mode: u16,
    sd_reserved: [u8; 6],
}

pub mod consts {
    pub const STREAM_STREAMS: &[u8] = b"Streams\0";
    pub const STREAM_CUSTOM_OBJECT_INFO: &[u8] = b"CustomObjectInfo\0";
    pub const STREAM_STRINGS: &[u8] = b"Strings\0";
    pub const STREAM_FILE_DATA: &[u8] = b"FileData\0";
    pub const STREAM_DIRECTORY_CONTENT: &[u8] = b"DirectoryContent\0";
    pub const STREAM_SYMLINK_TARGET: &[u8] = b"SymlinkTarget\0";
    pub const STREAM_DEVICEID: &[u8] = b"DeviceId\0";
    pub const STREAM_LEGACY_DEVICE_NUMBER: &[u8] = b"LegacyDeviceNumber\0";
    pub const STREAM_SECURITY_DESCRIPTOR: &[u8] = b"SecurityDescriptor\0";

    pub const PHANTOMFS_MAGIC: [u8; 4] = *b"\x0FSPh";

    pub const MAJOR_VERSION: u32 = 1;
    pub const MINOR_VERSION: u32 = 0;
    pub const REVISION: u32 = 0;
}

bitflags::bitflags! {
    #[repr(transparent)]
    #[derive(Zeroable, Pod)]
    pub struct FSFeatures: u64 {

    }
}

bitflags::bitflags! {
    #[repr(transparent)]
    #[derive(Zeroable, Pod)]
    pub struct FSROFeatures: u64 {

    }
}

#[repr(C, align(64))]
#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq, Zeroable, Pod)]
pub struct RootFSDescriptor {
    magic: [u8; 4],
    major: u32,
    minor: u32,
    revision: u32,
    partid: u128,
    features: FSFeatures,
    rofeatures: FSROFeatures,
    objtab: u128,
    objtabsize: u64,
    rootidx: u64,
    partnameidx: Option<NonZeroU64>,
    partname: [u8; 24],
    reserved112: [u8; 8],
    descriptor_size: u32,
    descriptor_crc: u32,
}

pub struct PhantomFS<S> {
    stream: S,
    descriptor: Option<RootFSDescriptor>,
}

impl<S> PhantomFS<S> {
    pub const fn new(inner: S) -> Self {
        Self {
            stream: inner,
            descriptor: None,
        }
    }

    pub fn into_inner(self) -> S {
        self.stream
    }

    pub fn create_new_fs(&mut self, partid: u128) {
        let mut desc = RootFSDescriptor {
            magic: consts::PHANTOMFS_MAGIC,
            major: consts::MAJOR_VERSION,
            minor: consts::MINOR_VERSION,
            revision: consts::REVISION,
            partid,
            descriptor_size: core::mem::size_of::<RootFSDescriptor>() as u32,
            ..Zeroable::zeroed()
        };

        let mut crc = crc_any::CRCu32::crc32();
        crc.digest(bytemuck::bytes_of(&desc));
        desc.descriptor_crc = crc.get_crc();

        self.descriptor = Some(desc);
    }
}

impl<S: Read + Seek> PhantomFS<S> {
    pub fn read_descriptor(&mut self) -> std::io::Result<()> {
        self.stream.seek(SeekFrom::Start(1024))?;
        let desc = self.descriptor.get_or_insert_with(Zeroable::zeroed);
        self.stream.read_exact(bytemuck::bytes_of_mut(desc))?;

        if desc.magic != consts::PHANTOMFS_MAGIC {
            return Err(io::Error::InvalidData(Some(alloc::format!(
                "Invalid Magic {:x?}",
                desc.magic
            ))));
        }

        Ok(())
    }

    pub fn get_or_read_descriptor(&mut self) -> std::io::Result<&mut RootFSDescriptor> {
        match &mut self.descriptor {
            Some(desc) => Ok(unsafe { &mut *(desc as *mut RootFSDescriptor) }),
            None => {
                self.read_descriptor()?;
                Ok(self.descriptor.as_mut().unwrap())
            }
        }
    }
}

impl<S: Read + Seek> Search for PhantomFS<S> {
    fn get_object_from(
        &mut self,
        pos: crate::traits::InodeId,
        pname: std::str::StringView,
    ) -> std::io::Result<crate::traits::ObjectId> {
        todo!()
    }

    fn get_stream_of_object(
        &mut self,
        obj: crate::traits::ObjectId,
        lname: std::str::StringView,
    ) -> std::io::Result<crate::traits::StreamId> {
        let desc = self.get_or_read_descriptor()?;

        Ok(StreamId(None))
    }
}

impl<S: Write + Seek> PhantomFS<S> {
    pub fn write_descriptor(&mut self) -> std::io::Result<()> {
        if let Some(desc) = &self.descriptor {
            self.stream.seek(SeekFrom::Start(1024))?;
            self.stream.write_all(bytemuck::bytes_of(desc))?;
        }
        Ok(())
    }
}
