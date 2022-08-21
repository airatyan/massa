// Copyright (c) 2022 MASSA LABS <info@massa.net>

//! This module implements a selector controller.
//! See `massa-pos-exports/controller_traits.rs` for functional details.

use crate::{Command, DrawCachePtr, InputDataPtr};
use anyhow::{bail, Result};
use massa_hash::Hash;
use massa_models::{api::IndexedSlot, prehash::Map, Address, Slot};
use massa_pos_exports::{PosResult, Selection, SelectorController, SelectorManager};
use tracing::{info, warn};

#[derive(Clone)]
/// implementation of the selector controller
pub struct SelectorControllerImpl {
    /// todo: use a config structure
    pub(crate) periods_per_cycle: u64,
    /// Cache storing the computed selections for each cycle.
    pub(crate) cache: DrawCachePtr, //TODO remove btreemap, make access constant-time
    /// Sync-able equivalent of std::mpsc for command sending
    pub(crate) input_data: InputDataPtr,
    /// thread count
    pub(crate) thread_count: u8,
}

impl SelectorController for SelectorControllerImpl {
    /// Feed cycle to the selector
    ///
    /// # Arguments
    /// * `cycle`: cycle number to be drawn
    /// * `lookback_rolls`: lookback rolls used for the draw (cycle - 3)
    /// * `lookback_seed`: lookback seed hash for the draw (cycle - 2)
    fn feed_cycle(&self, cycle: u64, lookback_rolls: Map<Address, u64>, lookback_seed: Hash) {
        self.input_data.1.lock().push_back(Command::DrawInput {
            cycle,
            lookback_rolls,
            lookback_seed,
        });
        self.input_data.0.notify_one();
    }

    /// Get [Selection] computed for a slot:
    /// # Arguments
    /// * `slot`: target slot of the selection
    fn get_selection(&self, slot: Slot) -> Result<Selection> {
        let cycle = slot.get_cycle(self.periods_per_cycle);
        match self
            .cache
            .read()
            .get(cycle)
            .and_then(|selections| selections.draws.get(&slot))
        {
            Some(selection) => Ok(selection.clone()),
            None => bail!("error: selection not found for slot {}", slot),
        }
    }

    /// Get [Address] of the selected block producer for a given slot
    /// # Arguments
    /// * `slot`: target slot of the selection
    fn get_producer(&self, slot: Slot) -> Result<Address> {
        Ok(self.get_selection(slot)?.producer)
    }

    /// Return a list of slots where `address` has been choosen to produce a
    /// block and a list where he is choosen for the endorsements.
    /// Look from the `start` slot to the `end` slot.
    fn get_address_selections(
        &self,
        address: &Address,
        mut slot: Slot, /* starting slot */
        end: Slot,
    ) -> (Vec<Slot>, Vec<IndexedSlot>) {
        let cache = self.cache.read();
        let mut slot_producers = vec![];
        let mut slot_endorsers = vec![];
        while slot < end {
            if let Some(selection) = cache
                .get(slot.get_cycle(self.periods_per_cycle))
                .and_then(|selections| selections.draws.get(&slot))
            {
                if selection.producer == *address {
                    slot_producers.push(slot);
                } else if let Some(index) = selection.endorsements.iter().position(|e| e == address)
                {
                    slot_endorsers.push(IndexedSlot { slot, index });
                }
            }
            slot = match slot.get_next_slot(self.thread_count) {
                Ok(next_slot) => next_slot,
                _ => break,
            };
        }
        (slot_producers, slot_endorsers)
    }

    /// Returns a boxed clone of self.
    /// Allows cloning `Box<dyn SelectorController>`,
    /// see `massa-pos-exports/controller_traits.rs`
    fn clone_box(&self) -> Box<dyn SelectorController> {
        Box::new(self.clone())
    }
}

/// Implementation of the Selector manager
/// Allows stopping the selector worker
pub struct SelectorManagerImpl {
    /// handle used to join the worker thread
    pub(crate) thread_handle: Option<std::thread::JoinHandle<PosResult<()>>>,
    /// Input data pointer (used to stop the selector thread)
    pub(crate) input_data: InputDataPtr,
}

impl SelectorManager for SelectorManagerImpl {
    /// stops the worker
    fn stop(&mut self) {
        info!("stopping selector worker...");
        {
            self.input_data.1.lock().push_front(Command::Stop);
            self.input_data.0.notify_one();
        }
        // join the selector thread
        if let Some(join_handle) = self.thread_handle.take() {
            if let Err(err) = join_handle
                .join()
                .expect("selector thread panicked on try to join")
            {
                warn!("{}", err);
            }
        }
        info!("selector worker stopped");
    }
}
