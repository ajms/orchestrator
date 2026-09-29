use orch_config::{PortBlock, PortRange};
use orch_core::SessionId;
use rusqlite::{OptionalExtension, Row, params};

use crate::{Store, StoreError};

pub(crate) fn port_block_from(row: &Row) -> rusqlite::Result<Option<PortBlock>> {
    let base: Option<u16> = row.get("port_base")?;
    let size: Option<u16> = row.get("port_size")?;
    Ok(base.zip(size).map(|(base, size)| PortBlock { base, size }))
}

impl Store {
    pub fn allocate_port_block(
        &mut self,
        session: &SessionId,
        range: &PortRange,
    ) -> Result<PortBlock, StoreError> {
        let tx = self.conn.transaction()?;
        let current = tx
            .query_row(
                "SELECT port_base, port_size FROM sessions WHERE id = ?1",
                params![session.as_str()],
                port_block_from,
            )
            .optional()?
            .ok_or(StoreError::UnknownSession)?;
        if let Some(block) = current {
            return Ok(block);
        }
        let reserved: Vec<PortBlock> = tx
            .prepare("SELECT port_base, port_size FROM sessions")?
            .query_map([], port_block_from)?
            .filter_map(Result::transpose)
            .collect::<rusqlite::Result<_>>()?;
        let block = range
            .blocks()
            .find(|candidate| !reserved.iter().any(|held| held.overlaps(*candidate)))
            .ok_or(StoreError::PortsExhausted)?;
        tx.execute(
            "UPDATE sessions SET port_base = ?2, port_size = ?3 WHERE id = ?1",
            params![session.as_str(), block.base, block.size],
        )?;
        tx.commit()?;
        Ok(block)
    }

    pub fn hold_port_block(
        &mut self,
        session: &SessionId,
        block: PortBlock,
    ) -> Result<Vec<SessionId>, StoreError> {
        let clashes = self
            .sessions()?
            .into_iter()
            .filter(|other| {
                other.id != *session
                    && !other.phase.is_terminal()
                    && other.port_block.is_some_and(|held| held.overlaps(block))
            })
            .map(|other| other.id)
            .collect();
        let changed = self.conn.execute(
            "UPDATE sessions SET port_base = ?2, port_size = ?3 WHERE id = ?1",
            params![session.as_str(), block.base, block.size],
        )?;
        match changed {
            0 => Err(StoreError::UnknownSession),
            _ => Ok(clashes),
        }
    }

    pub fn free_port_block(&mut self, session: &SessionId) -> Result<(), StoreError> {
        let changed = self.conn.execute(
            "UPDATE sessions SET port_base = NULL, port_size = NULL WHERE id = ?1",
            params![session.as_str()],
        )?;
        match changed {
            0 => Err(StoreError::UnknownSession),
            _ => Ok(()),
        }
    }

    pub fn rebuild_port_blocks(&mut self) -> Result<Vec<(SessionId, PortBlock)>, StoreError> {
        let mut reserved = Vec::new();
        for session in self.sessions()? {
            let Some(block) = session.port_block else {
                continue;
            };
            if session.phase.is_terminal() {
                self.free_port_block(&session.id)?;
            } else {
                reserved.push((session.id, block));
            }
        }
        Ok(reserved)
    }
}
