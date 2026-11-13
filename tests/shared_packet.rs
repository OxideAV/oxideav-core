//! Shared-payload packets through the container / codec traits: the
//! provided methods bridge to the `Packet` ones, and an implementation
//! that overrides them hands out or keeps views without copying.

use oxideav_core::{
    CodecId, Decoder, Demuxer, Error, Frame, Muxer, Packet, PacketSource, Result, SharedBytes,
    SharedPacket, StreamInfo, TimeBase,
};

fn tb() -> TimeBase {
    TimeBase::new(1, 1000)
}

/// Knows nothing about shared packets.
struct PlainDemuxer(u8);

impl Demuxer for PlainDemuxer {
    fn format_name(&self) -> &str {
        "plain"
    }
    fn streams(&self) -> &[StreamInfo] {
        &[]
    }
    fn next_packet(&mut self) -> Result<Packet> {
        if self.0 == 3 {
            return Err(Error::Eof);
        }
        self.0 += 1;
        Ok(Packet::new(0, tb(), vec![self.0; 4]).with_pts(i64::from(self.0)))
    }
}

impl PacketSource for PlainDemuxer {
    fn streams(&self) -> &[StreamInfo] {
        &[]
    }
    fn next_packet(&mut self) -> Result<Packet> {
        Demuxer::next_packet(self)
    }
}

/// Holds one buffer and hands out views of it.
struct ViewDemuxer {
    file: SharedBytes,
    next: usize,
}

impl ViewDemuxer {
    const ITEM: usize = 8;
}

impl Demuxer for ViewDemuxer {
    fn format_name(&self) -> &str {
        "view"
    }
    fn streams(&self) -> &[StreamInfo] {
        &[]
    }
    fn next_packet(&mut self) -> Result<Packet> {
        self.next_shared_packet().map(SharedPacket::into_packet)
    }
    fn next_shared_packet(&mut self) -> Result<SharedPacket> {
        let start = self.next * Self::ITEM;
        let view = self
            .file
            .try_slice(start..start + Self::ITEM)
            .ok_or(Error::Eof)?;
        self.next += 1;
        Ok(SharedPacket::new(0, tb(), view).with_pts(self.next as i64))
    }
}

/// Knows nothing about shared packets; records what it was fed.
#[derive(Default)]
struct PlainDecoder {
    id: Option<CodecId>,
    fed: Vec<Vec<u8>>,
}

impl Decoder for PlainDecoder {
    fn codec_id(&self) -> &CodecId {
        self.id.as_ref().unwrap()
    }
    fn send_packet(&mut self, packet: &Packet) -> Result<()> {
        self.fed.push(packet.data.clone());
        Ok(())
    }
    fn receive_frame(&mut self) -> Result<Frame> {
        Err(Error::NeedMore)
    }
    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

/// Keeps the views it is fed.
struct ViewDecoder {
    id: CodecId,
    kept: Vec<SharedBytes>,
}

impl Decoder for ViewDecoder {
    fn codec_id(&self) -> &CodecId {
        &self.id
    }
    fn send_packet(&mut self, packet: &Packet) -> Result<()> {
        self.send_shared_packet(packet.clone().into_shared())
    }
    fn send_shared_packet(&mut self, packet: SharedPacket) -> Result<()> {
        self.kept.push(packet.data);
        Ok(())
    }
    fn receive_frame(&mut self) -> Result<Frame> {
        Err(Error::NeedMore)
    }
    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

#[derive(Default)]
struct PlainMuxer(Vec<u8>);

impl Muxer for PlainMuxer {
    fn format_name(&self) -> &str {
        "plain"
    }
    fn write_header(&mut self) -> Result<()> {
        Ok(())
    }
    fn write_packet(&mut self, packet: &Packet) -> Result<()> {
        self.0.extend_from_slice(&packet.data);
        Ok(())
    }
    fn write_trailer(&mut self) -> Result<()> {
        Ok(())
    }
}

#[test]
fn default_next_shared_packet_wraps_next_packet() {
    let mut d: Box<dyn Demuxer> = Box::new(PlainDemuxer(0));
    let a = d.next_shared_packet().unwrap();
    assert_eq!(a.data, vec![1u8; 4]);
    assert_eq!(a.pts, Some(1));
    // Both methods advance the same sequence.
    let b = d.next_packet().unwrap();
    assert_eq!(b.data, vec![2u8; 4]);
    let c = d.next_shared_packet().unwrap();
    assert_eq!(c.data, vec![3u8; 4]);
    assert!(matches!(d.next_shared_packet(), Err(Error::Eof)));

    let mut s: Box<dyn PacketSource> = Box::new(PlainDemuxer(2));
    assert_eq!(s.next_shared_packet().unwrap().data, vec![3u8; 4]);
    assert!(matches!(s.next_shared_packet(), Err(Error::Eof)));
}

#[test]
fn overriding_demuxer_and_decoder_pass_views_end_to_end() {
    let file = SharedBytes::from((0u8..64).collect::<Vec<u8>>());
    let mut d: Box<dyn Demuxer> = Box::new(ViewDemuxer {
        file: file.clone(),
        next: 0,
    });
    let mut dec = ViewDecoder {
        id: CodecId::new("view"),
        kept: Vec::new(),
    };
    let mut n = 0usize;
    loop {
        match d.next_shared_packet() {
            Ok(p) => {
                assert!(p.data.shares_buffer_with(&file));
                assert_eq!(p.data.as_ptr(), file[n * 8..].as_ptr());
                (&mut dec as &mut dyn Decoder)
                    .send_shared_packet(p)
                    .unwrap();
                n += 1;
            }
            Err(Error::Eof) => break,
            Err(e) => panic!("{e:?}"),
        }
    }
    assert_eq!(n, 8);
    for (i, kept) in dec.kept.iter().enumerate() {
        assert_eq!(kept.as_ptr(), file[i * 8..].as_ptr());
        assert_eq!(kept.range(), i * 8..(i + 1) * 8);
        // The surrounding buffer is reachable from the view.
        assert_eq!(kept.buffer().len(), 64);
    }
}

#[test]
fn view_packets_reach_components_that_only_know_packet() {
    let file = SharedBytes::from((0u8..16).collect::<Vec<u8>>());
    let mut d = ViewDemuxer { file, next: 0 };
    // Old-style consumer of a view-producing demuxer.
    let first = d.next_packet().unwrap();
    assert_eq!(first.data, (0u8..8).collect::<Vec<u8>>());

    // View-producing demuxer into a decoder and a muxer that only
    // implement the `Packet` methods.
    let second = d.next_shared_packet().unwrap();
    let mut dec = PlainDecoder::default();
    dec.send_shared_packet(second.clone()).unwrap();
    assert_eq!(dec.fed, vec![(8u8..16).collect::<Vec<u8>>()]);
    let mut mux = PlainMuxer::default();
    mux.write_shared_packet(second).unwrap();
    assert_eq!(mux.0, (8u8..16).collect::<Vec<u8>>());

    // A plain packet into a view-keeping decoder.
    let mut view_dec = ViewDecoder {
        id: CodecId::new("view"),
        kept: Vec::new(),
    };
    view_dec.send_packet(&first).unwrap();
    assert_eq!(view_dec.kept[0], first.data);
}
