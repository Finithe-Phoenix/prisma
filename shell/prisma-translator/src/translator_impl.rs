#[cfg(all(windows, target_arch = "arm64ec"))]
fn arm64ec_phase_marker(message: &'static [u8]) {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetStdHandle(kind: u32) -> *mut std::ffi::c_void;
        fn WriteFile(
            file: *mut std::ffi::c_void,
            buffer: *const std::ffi::c_void,
            bytes_to_write: u32,
            bytes_written: *mut u32,
            overlapped: *mut std::ffi::c_void,
        ) -> i32;
    }

    const STD_ERROR_HANDLE: u32 = (-12_i32) as u32;
    let Ok(bytes_to_write) = u32::try_from(message.len()) else {
        return;
    };
    // SAFETY: diagnostics borrow a static message and never own the handle.
    let file = unsafe { GetStdHandle(STD_ERROR_HANDLE) };
    if file.is_null() || file.addr() == usize::MAX {
        return;
    }
    let mut bytes_written = 0_u32;
    let _ = unsafe {
        WriteFile(
            file,
            message.as_ptr().cast(),
            bytes_to_write,
            &raw mut bytes_written,
            std::ptr::null_mut(),
        )
    };
}

impl Translator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Translate the guest instruction at `bytes[0..]` (decoded at `guest_addr`)
    /// into ARM64 machine code, running the full optimization pipeline and
    /// memoizing the result in the translation cache.
    ///
    /// # Errors
    /// [`TranslateError::Decode`] if the bytes are not a decodable instruction;
    /// [`TranslateError::Lower`] if the resulting IR is not lowerable by the
    /// current backend slice.
    pub fn translate(
        &mut self,
        guest_addr: u64,
        bytes: &[u8],
    ) -> Result<Translation, TranslateError> {
        let decoded = decode_one_at(bytes, 0, guest_addr).map_err(TranslateError::Decode)?;
        self.translate_decoded(guest_addr, bytes, &decoded)
    }

    /// Translate exactly one instruction for an execution loop that resolves
    /// control flow dynamically and owns its executable-code cache. This path
    /// therefore builds neither CFG successors nor a duplicate cache entry.
    ///
    /// # Errors
    /// [`TranslateError`] if the instruction cannot be decoded or lowered.
    pub fn translate_dispatch_instruction(
        &mut self,
        guest_addr: u64,
        bytes: &[u8],
    ) -> Result<(Translation, bool), TranslateError> {
        #[cfg(all(windows, target_arch = "arm64ec"))]
        arm64ec_phase_marker(b"prisma-phase: translator-decode-enter\n");
        let decoded = decode_one_at(bytes, 0, guest_addr).map_err(TranslateError::Decode)?;
        #[cfg(all(windows, target_arch = "arm64ec"))]
        arm64ec_phase_marker(b"prisma-phase: translator-decode-ready\n");
        let Some(_) = bytes.get(..decoded.bytes_consumed) else {
            return Err(TranslateError::Truncated {
                offset: 0,
                consumed: decoded.bytes_consumed,
                remaining: bytes.len(),
            });
        };
        let ended_at_terminator = decoded.stmts.iter().any(|stmt| is_terminator(&stmt.op));
        #[cfg(all(windows, target_arch = "arm64ec"))]
        arm64ec_phase_marker(b"prisma-phase: translator-lower-enter\n");
        let translation = self.translate_decoded_uncached(&decoded)?;
        Ok((translation, ended_at_terminator))
    }

    /// Translate a straight-line run of instructions starting at `guest_addr`
    /// into one concatenated ARM64 block, stopping at the first control-transfer
    /// instruction, when `bytes` is exhausted, or after `max_insns` (a guard
    /// against pathological runs). Each instruction is translated and cached
    /// independently. An undecodable byte mid-run ends the block; an
    /// undecodable first instruction is an error.
    ///
    /// # Errors
    /// [`TranslateError`] from the first instruction if it cannot be decoded or
    /// lowered.
    pub fn translate_block(
        &mut self,
        guest_addr: u64,
        bytes: &[u8],
        max_insns: usize,
    ) -> Result<BlockTranslation, TranslateError> {
        if max_insns == 1 && !bytes.is_empty() {
            let decoded = decode_one_at(bytes, 0, guest_addr).map_err(TranslateError::Decode)?;
            let Some(insn) = bytes.get(..decoded.bytes_consumed) else {
                return Err(TranslateError::Truncated {
                    offset: 0,
                    consumed: decoded.bytes_consumed,
                    remaining: bytes.len(),
                });
            };
            let terminator = decoded.stmts.iter().find(|stmt| is_terminator(&stmt.op));
            let ended_at_terminator = terminator.is_some();
            let successors = terminator.map_or_else(Vec::new, |stmt| static_successors(&stmt.op));
            let translation = self.translate_decoded(guest_addr, insn, &decoded)?;
            return Ok(BlockTranslation {
                code: translation.code,
                instruction_count: 1,
                guest_bytes: decoded.bytes_consumed,
                ended_at_terminator,
                successors: if ended_at_terminator {
                    successors
                } else {
                    vec![guest_addr.wrapping_add(decoded.bytes_consumed as u64)]
                },
            });
        }

        let mut code = Vec::new();
        let mut offset = 0usize;
        let mut pc = guest_addr;
        let mut instruction_count = 0usize;
        let mut ended_at_terminator = false;
        let mut successors: Vec<u64> = Vec::new();

        while offset < bytes.len() && instruction_count < max_insns {
            let decoded = match decode_one_at(bytes, offset, pc) {
                Ok(d) => d,
                Err(e) => {
                    if instruction_count == 0 {
                        return Err(TranslateError::Decode(e));
                    }
                    break;
                }
            };
            // The decoder can report consuming more bytes than remain (a
            // truncated trailing instruction whose operand runs off the
            // buffer). Bound the slice instead of panicking: stop the block if
            // we already have an instruction, else surface a typed error.
            let Some(insn) = offset
                .checked_add(decoded.bytes_consumed)
                .and_then(|end| bytes.get(offset..end))
            else {
                if instruction_count == 0 {
                    return Err(TranslateError::Truncated {
                        offset,
                        consumed: decoded.bytes_consumed,
                        remaining: bytes.len() - offset,
                    });
                }
                break;
            };
            let translation = self.translate_decoded(pc, insn, &decoded)?;
            code.extend_from_slice(&translation.code);
            instruction_count += 1;
            offset += decoded.bytes_consumed;
            pc = pc.wrapping_add(decoded.bytes_consumed as u64);
            if let Some(term) = decoded.stmts.iter().find(|s| is_terminator(&s.op)) {
                successors = static_successors(&term.op);
                ended_at_terminator = true;
                break;
            }
        }

        if !ended_at_terminator && instruction_count > 0 {
            // Block ended on the cap / exhausted bytes: its only successor is
            // the fall-through PC after the last instruction.
            successors = vec![pc];
        }

        Ok(BlockTranslation {
            code,
            instruction_count,
            guest_bytes: offset,
            ended_at_terminator,
            successors,
        })
    }

    /// Like [`Translator::translate_block`], but fuses the whole straight-line
    /// run into a SINGLE optimized SSA region instead of translating each
    /// instruction in isolation. The decoder numbers refs per instruction, so
    /// each instruction's refs are renumbered (via [`prisma_ir::Op::map_refs`])
    /// into a disjoint range before being concatenated; the default pipeline
    /// then optimizes ACROSS instruction boundaries (e.g. forwarding a
    /// register write into a later read) and the result is lowered once.
    ///
    /// Not cached (the unit is a block, not a single instruction). Returns an
    /// empty block for empty input.
    ///
    /// # Errors
    /// [`TranslateError::Decode`] if the first instruction cannot be decoded;
    /// [`TranslateError::Lower`] if the fused region is not lowerable.
    pub fn translate_fused_block(
        &mut self,
        guest_addr: u64,
        bytes: &[u8],
        max_insns: usize,
    ) -> Result<BlockTranslation, TranslateError> {
        let opt = self.optimize_fused_block(guest_addr, bytes, max_insns)?;
        // The runtime executes every translated block via execute_block, which
        // wraps it in the AAPCS64 block prologue/epilogue. A terminator (guest
        // ret, SYSCALL) must route through the full epilogue — a bare ret would
        // skip the prologue's stack/callee-saved restore and corrupt the host on
        // return. with_branch_exits additionally routes a relative branch through
        // the frame's next_pc (this is a single block, with no sibling to branch
        // to) so the run loop can chain to the taken target.
        let words = Lowerer::new()
            .with_branch_exits()
            .lower_function(&opt.func)
            .map_err(TranslateError::Lower)?;
        let code = encode_words(&words);

        Ok(BlockTranslation {
            code,
            instruction_count: opt.instruction_count,
            guest_bytes: opt.guest_bytes,
            ended_at_terminator: opt.ended_at_terminator,
            successors: opt.successors,
        })
    }

    /// Decode a straight-line run starting at `guest_addr` into one block, shift
    /// each instruction's SSA refs above the previous so names never collide, and
    /// run the optimization pipeline — returning the optimized IR plus how the
    /// block ended, WITHOUT lowering. [`Self::translate_fused_block`] lowers this;
    /// the reference interpreter ([`crate::interp`]) executes the same IR, so the
    /// backend and the oracle agree on exactly what they evaluate.
    ///
    /// # Errors
    /// [`TranslateError::Decode`] if the first instruction is undecodable.
    pub fn optimize_fused_block(
        &mut self,
        guest_addr: u64,
        bytes: &[u8],
        max_insns: usize,
    ) -> Result<OptimizedBlock, TranslateError> {
        let mut stmts = Vec::new();
        let mut offset = 0usize;
        let mut pc = guest_addr;
        let mut instruction_count = 0usize;
        let mut ended_at_terminator = false;
        let mut successors: Vec<u64> = Vec::new();
        // Next free SSA ref: every instruction's refs are shifted above all
        // refs already placed in the block so names never collide.
        let mut base: u32 = 0;

        while offset < bytes.len() && instruction_count < max_insns {
            let decoded = match decode_one_at(bytes, offset, pc) {
                Ok(d) => d,
                Err(e) => {
                    if instruction_count == 0 {
                        return Err(TranslateError::Decode(e));
                    }
                    break;
                }
            };

            // As in `translate_block`, reject a decoded instruction whose
            // reported size runs past the remaining guest buffer. A truncated
            // trailing instruction must not inflate `guest_bytes` beyond the
            // bytes supplied by the caller.
            let Some(end) = offset
                .checked_add(decoded.bytes_consumed)
                .filter(|&end| end <= bytes.len())
            else {
                if instruction_count == 0 {
                    return Err(TranslateError::Truncated {
                        offset,
                        consumed: decoded.bytes_consumed,
                        remaining: bytes.len() - offset,
                    });
                }
                break;
            };

            // Optimize one instruction at a time before fusion. Each decoded
            // instruction ends by publishing its architectural state; keeping
            // those stores prevents cross-instruction SSA values from hiding
            // live guest registers from Wine exception recovery at GuestPc.
            let optimized_instruction = self.pipeline.run(Function {
                entry: 0,
                blocks: vec![BasicBlock {
                    id: 0,
                    stmts: decoded.stmts.clone(),
                }],
            });
            let mut renumbered = optimized_instruction
                .blocks
                .into_iter()
                .flat_map(|block| block.stmts)
                .collect::<Vec<_>>();
            let mut local_max = base;
            let mut overflow = false;
            for stmt in &mut renumbered {
                stmt.map_refs(|r| {
                    r.checked_add(base).map_or_else(
                        || {
                            overflow = true;
                            r
                        },
                        |v| {
                            local_max = local_max.max(v);
                            v
                        },
                    )
                });
            }
            if overflow {
                // Ref space exhausted (pathological run): stop cleanly.
                break;
            }

            // Publish the exact x64 instruction boundary before its effects.
            // The ARM64EC exception bridge reads this marker from the shared
            // state frame, so fused native blocks retain precise Wine SEH PCs.
            stmts.push(Stmt::new(None, Op::GuestPc(GuestPc { pc })));
            stmts.extend(renumbered);
            instruction_count += 1;
            offset = end;
            pc = pc.wrapping_add(decoded.bytes_consumed as u64);
            base = local_max.saturating_add(1);

            if let Some(term) = decoded.stmts.iter().find(|s| is_terminator(&s.op)) {
                successors = static_successors(&term.op);
                ended_at_terminator = true;
                break;
            }
        }

        if !ended_at_terminator && instruction_count > 0 {
            successors = vec![pc];
        }

        let func = Function {
            entry: 0,
            blocks: vec![BasicBlock { id: 0, stmts }],
        };
        Ok(OptimizedBlock {
            func,
            instruction_count,
            guest_bytes: offset,
            ended_at_terminator,
            successors,
        })
    }

    fn translate_decoded(
        &mut self,
        guest_addr: u64,
        insn: &[u8],
        decoded: &Decoded,
    ) -> Result<Translation, TranslateError> {
        if let LookupResult::Hit(entry) = self.cache.lookup(guest_addr, insn) {
            self.stats.cache_hits += 1;
            return Ok(Translation {
                code: entry.code_bytes.into_vec(),
                guest_bytes: decoded.bytes_consumed,
                from_cache: true,
            });
        }
        self.stats.cache_misses += 1;

        let code = self.lower_decoded(decoded)?;
        let entry = CacheEntry {
            guest_addr,
            guest_size: u32::try_from(decoded.bytes_consumed).unwrap_or(u32::MAX),
            code_size: u32::try_from(code.len()).unwrap_or(u32::MAX),
            code_bytes: code.clone().into_boxed_slice(),
            hit_count: 0,
            last_used: 0,
        };
        self.cache.insert((guest_addr, fnv1a_64(insn)), entry);

        Ok(Translation {
            code,
            guest_bytes: decoded.bytes_consumed,
            from_cache: false,
        })
    }

    fn translate_decoded_uncached(
        &self,
        decoded: &Decoded,
    ) -> Result<Translation, TranslateError> {
        let code = self.lower_decoded(decoded)?;
        Ok(Translation {
            code,
            guest_bytes: decoded.bytes_consumed,
            from_cache: false,
        })
    }

    fn lower_decoded(&self, decoded: &Decoded) -> Result<Vec<u8>, TranslateError> {
        #[cfg(all(windows, target_arch = "arm64ec"))]
        arm64ec_phase_marker(b"prisma-phase: translator-ir-enter\n");
        let func = Function {
            entry: 0,
            blocks: vec![BasicBlock {
                id: 0,
                stmts: decoded.stmts.clone(),
            }],
        };
        #[cfg(all(windows, target_arch = "arm64ec"))]
        arm64ec_phase_marker(b"prisma-phase: translator-ir-ready\n");
        let optimized = self.pipeline.run(func);
        #[cfg(all(windows, target_arch = "arm64ec"))]
        arm64ec_phase_marker(b"prisma-phase: translator-pipeline-ready\n");

        // The runtime executes every translated block via execute_block, which
        // wraps it in the AAPCS64 block prologue/epilogue. A terminator (guest
        // ret, SYSCALL) must route through the full epilogue — a bare ret would
        // skip the prologue's stack/callee-saved restore and corrupt the host on
        // return. with_branch_exits additionally routes a relative branch through
        // the frame's next_pc (this is a single block, with no sibling to branch
        // to) so the run loop can chain to the taken target.
        let words = Lowerer::new()
            .with_branch_exits()
            .lower_function(&optimized)
            .map_err(TranslateError::Lower)?;
        #[cfg(all(windows, target_arch = "arm64ec"))]
        arm64ec_phase_marker(b"prisma-phase: translator-lowerer-ready\n");
        let code = encode_words(&words);
        #[cfg(all(windows, target_arch = "arm64ec"))]
        arm64ec_phase_marker(b"prisma-phase: translator-encode-ready\n");
        Ok(code)
    }

    /// Number of distinct translations currently held in the cache.
    pub fn cached_count(&self) -> usize {
        self.cache.entry_count()
    }

    /// Cumulative cache hit/miss counters since construction (or last reset).
    pub const fn stats(&self) -> TranslatorStats {
        self.stats
    }

    /// Reset the hit/miss counters to zero (the cache contents are untouched).
    pub fn reset_stats(&mut self) {
        self.stats = TranslatorStats::default();
    }

    /// Bound the translation cache: at most `max_entries` entries and
    /// `max_bytes` of code (0 means unbounded). LRU eviction enforces both.
    pub fn set_cache_limits(&mut self, max_entries: usize, max_bytes: usize) {
        self.cache.set_limits(max_entries, max_bytes);
    }

    /// Drop the cached translation(s) at `guest_addr`. Call this when the guest
    /// rewrites code at that address (self-modifying code) so the next
    /// translation re-decodes the new bytes instead of serving stale code.
    pub fn invalidate(&mut self, guest_addr: u64) {
        // The cache keys on (addr, content hash) but tracks addr -> hash, so a
        // zero-hash key evicts whatever translation currently lives at the addr.
        self.cache.invalidate(&(guest_addr, 0));
    }

    /// Drop every cached translation (e.g. on a full guest address-space flush).
    pub fn clear_cache(&mut self) {
        self.cache.clear();
    }
}

fn encode_words(words: &[u32]) -> Vec<u8> {
    let mut code = vec![0; std::mem::size_of_val(words)];
    let (chunks, remainder) = code.as_chunks_mut::<4>();
    debug_assert_eq!(remainder, []);
    for (bytes, word) in chunks.iter_mut().zip(words) {
        bytes.copy_from_slice(&word.to_le_bytes());
    }
    code
}
