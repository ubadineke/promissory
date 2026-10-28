//! In-memory compact block cache for `zcash_client_backend::sync::run`.

use std::collections::BTreeMap;
use std::sync::Mutex;

use async_trait::async_trait;
use zcash_client_backend::{
    data_api::{
        chain::{error, BlockCache, BlockSource},
        scanning::ScanRange,
    },
    proto::compact_formats::CompactBlock,
};
use zcash_protocol::consensus::BlockHeight;

#[derive(Default)]
pub struct MemBlockCache {
    blocks: Mutex<BTreeMap<u32, CompactBlock>>,
}

fn height_of(block: &CompactBlock) -> u32 {
    block.height as u32
}

impl BlockSource for MemBlockCache {
    type Error = std::io::Error;

    fn with_blocks<F, WalletErrT>(
        &self,
        from_height: Option<BlockHeight>,
        limit: Option<usize>,
        mut with_block: F,
    ) -> Result<(), error::Error<WalletErrT, Self::Error>>
    where
        F: FnMut(CompactBlock) -> Result<(), error::Error<WalletErrT, Self::Error>>,
    {
        let from = from_height.map(u32::from).unwrap_or(0);
        let blocks: Vec<CompactBlock> = self
            .blocks
            .lock()
            .unwrap()
            .range(from..)
            .take(limit.unwrap_or(usize::MAX))
            .map(|(_, b)| b.clone())
            .collect();
        for block in blocks {
            with_block(block)?;
        }
        Ok(())
    }
}

#[async_trait]
impl BlockCache for MemBlockCache {
    fn get_tip_height(&self, range: Option<&ScanRange>) -> Result<Option<BlockHeight>, Self::Error> {
        let blocks = self.blocks.lock().unwrap();
        let tip = match range {
            Some(r) => blocks
                .range(u32::from(r.block_range().start)..u32::from(r.block_range().end))
                .next_back()
                .map(|(h, _)| *h),
            None => blocks.keys().next_back().copied(),
        };
        Ok(tip.map(BlockHeight::from_u32))
    }

    async fn read(&self, range: &ScanRange) -> Result<Vec<CompactBlock>, Self::Error> {
        Ok(self
            .blocks
            .lock()
            .unwrap()
            .range(u32::from(range.block_range().start)..u32::from(range.block_range().end))
            .map(|(_, b)| b.clone())
            .collect())
    }

    async fn insert(&self, compact_blocks: Vec<CompactBlock>) -> Result<(), Self::Error> {
        let mut blocks = self.blocks.lock().unwrap();
        for b in compact_blocks {
            blocks.insert(height_of(&b), b);
        }
        Ok(())
    }

    async fn delete(&self, range: ScanRange) -> Result<(), Self::Error> {
        let mut blocks = self.blocks.lock().unwrap();
        let start = u32::from(range.block_range().start);
        let end = u32::from(range.block_range().end);
        blocks.retain(|h, _| *h < start || *h >= end);
        Ok(())
    }
}
