mod path;
mod segment;
use path::PathType;
use segment::SegmentType;

use super::{
    byte_representation::{BytesLookup, RlcLookup},
    key_bit::KeyBitLookup,
    one_hot::OneHot,
    poseidon::PoseidonLookup,
};
use crate::{
    constraint_builder::{AdviceColumn, ConstraintBuilder, Query, SelectorColumn},
    serde::SMTTrace,
    types::Proof,
    MPTProofType,
};
use ethers_core::k256::elliptic_curve::PrimeField;
use ethers_core::types::Address;
use halo2_proofs::{
    arithmetic::FieldExt, circuit::Region, halo2curves::bn256::Fr, plonk::ConstraintSystem,
};
use strum::IntoEnumIterator;

pub trait MptUpdateLookup {
    fn lookup<F: FieldExt>(&self) -> [Query<F>; 7];
}

#[derive(Clone)]
struct MptUpdateConfig {
    selector: SelectorColumn,

    old_hash: AdviceColumn,
    new_hash: AdviceColumn,

    proof_key: AdviceColumn,

    old_value_rlc: AdviceColumn,
    new_value_rlc: AdviceColumn,

    proof_type: OneHot<MPTProofType>,

    address: AdviceColumn,
    storage_key_rlc: AdviceColumn,

    segment_type: OneHot<SegmentType>,
    path_type: OneHot<PathType>,
    depth: AdviceColumn,

    path_key: AdviceColumn,
    direction: AdviceColumn, // this actually must be binary because of a KeyBitLookup

    sibling: AdviceColumn,
}

impl MptUpdateLookup for MptUpdateConfig {
    fn lookup<F: FieldExt>(&self) -> [Query<F>; 7] {
        let is_root = || self.segment_type.matches(SegmentType::Start);
        let old_root = self.old_hash.current() * is_root();
        let new_root = self.new_hash.current() * is_root();
        // let proof_type = self
        //     .proof_type
        //     .iter()
        //     .enumerate()
        //     .map(|(i, column)| column.current() * i)
        //     .sum();
        let proof_type = Query::one();
        let old_value_rlc = self.new_value_rlc.current() * is_root();
        let new_value_rlc = self.old_value_rlc.current() * is_root();
        let address = self.address.current();
        let storage_key_rlc = self.storage_key_rlc.current();

        [
            old_root,
            new_root,
            old_value_rlc,
            new_value_rlc,
            proof_type,
            address,
            storage_key_rlc,
        ]
    }
}

impl MptUpdateConfig {
    fn configure<F: FieldExt>(
        cs: &mut ConstraintSystem<F>,
        cb: &mut ConstraintBuilder<F>,
        poseidon: &impl PoseidonLookup,
        key_bit: &impl KeyBitLookup,
        rlc: &impl RlcLookup,
        bytes: &impl BytesLookup,
    ) -> Self {
        let ([selector], [], [old_hash, new_hash]) = cb.build_columns(cs);

        let proof_type = OneHot::configure(cs, cb);
        let [address, storage_key_rlc] = cb.advice_columns(cs);

        let [old_value_rlc, new_value_rlc] = cb.advice_columns(cs);

        let [depth, proof_key, path_key, direction, sibling] = cb.advice_columns(cs);

        let segment_type = OneHot::configure(cs, cb);
        let path_type = OneHot::configure(cs, cb);

        // cb.add_lookup(
        //     "direction is correct for key and depth",
        //     [path_key.current(), depth.current(), direction.current()],
        //     key_bit.lookup(),
        // );

        // cb.add_lookup(
        //     "direction = key.bit(depth)",
        //     [path_key.current(), depth.current(), direction.current()],
        //     key_bit.lookup(),
        // );

        let config = Self {
            selector,
            proof_key,
            old_hash,
            new_hash,
            proof_type,
            old_value_rlc,
            new_value_rlc,
            address,
            storage_key_rlc,
            segment_type,
            path_type,
            path_key,
            depth,
            direction,
            sibling,
        };

        // Transitions for state machines:
        // TODO: rethink this justification later.... maybe we can just do the forward transitions?
        // We constrain backwards transitions (instead of the forward ones) because the
        // backwards transitions can be enabled on every row except the first (instead
        // of every row except the last). This makes the setting the selectors more
        // consistent between the tests, where the number of active rows is small,
        // and in production, where the number is much larger.
        for (sink, sources) in segment::backward_transitions().iter() {
            cb.condition(config.segment_type.matches(*sink), |cb| {
                cb.assert(
                    "backward transition for segment",
                    config.selector.current(),
                    config.segment_type.previous_in(&sources),
                );
            });
        }
        for (sink, sources) in path::backward_transitions().iter() {
            cb.condition(config.path_type.matches(*sink), |cb| {
                cb.assert(
                    "backward transition for path",
                    config.selector.current(),
                    config.path_type.previous_in(&sources),
                );
            });
        }
        // Depth increases by one iff segment type is unchanged, else it is 0?

        for variant in PathType::iter() {
            let conditional_constraints = |cb: &mut ConstraintBuilder<F>| match variant {
                PathType::Start => {} // TODO
                PathType::Common => configure_common_path(cb, &config, poseidon),
                PathType::ExtensionOld => configure_extension_old(cb, &config, poseidon),
                PathType::ExtensionNew => configure_extension_new(cb, &config, poseidon),
            };
            cb.condition(config.path_type.matches(variant), conditional_constraints);
        }

        for variant in MPTProofType::iter() {
            let conditional_constraints = |cb: &mut ConstraintBuilder<F>| match variant {
                MPTProofType::NonceChanged => configure_nonce(cb, &config, bytes),
                MPTProofType::BalanceChanged => configure_balance(cb, &config),
                MPTProofType::CodeHashExists => configure_code_hash(cb, &config),
                MPTProofType::AccountDoesNotExist => configure_empty_account(cb, &config),
                MPTProofType::AccountDestructed => configure_self_destruct(cb, &config),
                MPTProofType::StorageChanged => configure_storage(cb, &config),
                MPTProofType::StorageDoesNotExist => configure_empty_storage(cb, &config),
                MPTProofType::PoseidonCodeHashExists => todo!(),
                MPTProofType::CodeSizeExists => todo!(),
            };
            cb.condition(config.proof_type.matches(variant), conditional_constraints);
        }

        config
    }

    fn assign(&self, region: &mut Region<'_, Fr>, updates: &[SMTTrace]) {
        let randomness = Fr::from(123123u64); // TODOOOOOOO

        let mut offset = 0;
        for update in updates {
            let proof = Proof::from(update.clone());

            for (direction, old_hash, new_hash, sibling, is_padding_open, is_padding_close) in
                &proof.address_hash_traces
            {
                self.selector.enable(region, offset);
                self.address
                    .assign(region, offset, address_to_fr(proof.claim.address));
                // self.storage_key_rlc.assign(region, offset, rlc(proof.claim.storage_key, randomness));
                // self.new_value_rlc.assign(region, offset, ...)
                // self.old_value_rlc.assign(region, offset, ...)

                // self.is_common_path.assign(
                //     region,
                //     offset,
                //     !(*is_padding_open || *is_padding_close),
                // );
                self.segment_type
                    .assign(region, offset, SegmentType::AccountTrie);

                // let path_type = match (*is_padding_open, *is_padding_close) {
                //     (false, false) => PathType::Common,
                //     (false, true) => PathType::Old,
                //     (true, false) => PathType::New,
                //     (true, true) => unreachable!(),
                // };
                // self.path_type.assign(region, offset, path_type);

                self.sibling.assign(region, offset, *sibling);
                self.new_hash.assign(region, offset, *new_hash);
                self.old_hash.assign(region, offset, *old_hash);
                self.direction.assign(region, offset, *direction);

                offset += 1;
            }
        }
    }
}

fn old_left<F: FieldExt>(config: &MptUpdateConfig) -> Query<F> {
    config.direction.current() * config.old_hash.previous()
        + (Query::one() - config.direction.current()) * config.sibling.previous()
}

fn old_right<F: FieldExt>(config: &MptUpdateConfig) -> Query<F> {
    config.direction.current() * config.sibling.previous()
        + (Query::one() - config.direction.current()) * config.old_hash.previous()
}

fn new_left<F: FieldExt>(config: &MptUpdateConfig) -> Query<F> {
    config.direction.current() * config.new_hash.previous()
        + (Query::one() - config.direction.current()) * config.sibling.previous()
}

fn new_right<F: FieldExt>(config: &MptUpdateConfig) -> Query<F> {
    config.direction.current() * config.sibling.previous()
        + (Query::one() - config.direction.current()) * config.new_hash.previous()
}

fn address_to_fr(a: Address) -> Fr {
    let mut bytes = [0u8; 32];
    bytes[32 - 20..].copy_from_slice(a.as_bytes());
    bytes.reverse();
    Fr::from_repr(bytes).unwrap()
}

fn configure_common_path<F: FieldExt>(
    cb: &mut ConstraintBuilder<F>,
    config: &MptUpdateConfig,
    poseidon: &impl PoseidonLookup,
) {
    // cb.add_lookup(
    //     "poseidon hash correct for old path",
    //     [
    //         old_left(config),
    //         old_right(config),
    //         config.old_hash.current(),
    //     ],
    //     poseidon.lookup(),
    // );
    // cb.add_lookup(
    //     "poseidon hash correct for new path",
    //     [
    //         new_left(config),
    //         new_right(config),
    //         config.new_hash.current(),
    //     ],
    //     poseidon.lookup(),
    // );
}

fn configure_extension_old<F: FieldExt>(
    cb: &mut ConstraintBuilder<F>,
    config: &MptUpdateConfig,
    poseidon: &impl PoseidonLookup,
) {
    // cb.add_lookup(
    //     "poseidon hash correct for old path",
    //     [
    //         old_left(config),
    //         old_right(config),
    //         config.old_hash.current(),
    //     ],
    //     poseidon.lookup(),
    // );
    cb.add_constraint(
        "sibling is zero for extension path",
        config.selector.current(),
        config.sibling.current(),
    );
    cb.add_constraint(
        "new_hash unchanged for path_type=Old",
        config.selector.current(),
        config.new_hash.current() - config.new_hash.previous(),
    );
}

fn configure_extension_new<F: FieldExt>(
    cb: &mut ConstraintBuilder<F>,
    config: &MptUpdateConfig,
    poseidon: &impl PoseidonLookup,
) {
    cb.add_constraint(
        "old_hash unchanged for path_type=new",
        config.selector.current(),
        config.old_hash.current() - config.old_hash.previous(),
    );
    cb.add_constraint(
        "sibling is zero for extension path",
        config.selector.current(),
        config.sibling.current(),
    );
    // cb.add_lookup(
    //     "poseidon hash correct for new path",
    //     [
    //         new_left(config),
    //         new_right(config),
    //         config.new_hash.current(),
    //     ],
    //     poseidon.lookup(),
    // );
}

fn configure_nonce<F: FieldExt>(
    cb: &mut ConstraintBuilder<F>,
    config: &MptUpdateConfig,
    bytes: &impl BytesLookup,
) {
    for variant in SegmentType::iter() {
        let conditional_constraints = |cb: &mut ConstraintBuilder<F>| match variant {
            SegmentType::Start => {
                cb.add_constraint(
                    "depth is 0",
                    config.selector.current(),
                    config.depth.current(),
                );
            }
            SegmentType::AccountTrie => {
                cb.add_constraint(
                    "depth increased by 1",
                    config.selector.current(),
                    config.depth.delta() - Query::one(),
                );
            }
            SegmentType::AccountLeaf0 => {
                cb.assert(
                    "path_type is Common",
                    config.selector.current(),
                    config.path_type.matches(PathType::Common),
                );
                cb.add_constraint(
                    "depth is 0",
                    config.selector.current(),
                    config.depth.current(),
                );
                cb.add_constraint(
                    "direction is 0",
                    config.selector.current(),
                    config.direction.current(),
                );
                // add constraints that sibling = old_path_key and new_path_key
            }
            SegmentType::AccountLeaf1 => {
                cb.assert(
                    "path_type is Common",
                    config.selector.current(),
                    config.path_type.matches(PathType::Common),
                );
                cb.add_constraint(
                    "depth is 0",
                    config.selector.current(),
                    config.depth.current(),
                );
                cb.add_constraint(
                    "direction is 0",
                    config.selector.current(),
                    config.direction.current(),
                );
            }
            SegmentType::AccountLeaf2 => {
                cb.assert(
                    "path_type is Common",
                    config.selector.current(),
                    config.path_type.matches(PathType::Common),
                );
                cb.add_constraint(
                    "depth is 0",
                    config.selector.current(),
                    config.depth.current(),
                );
                cb.add_constraint(
                    "direction is 0",
                    config.selector.current(),
                    config.direction.current(),
                );
            }
            SegmentType::AccountLeaf3 => {
                cb.assert(
                    "path_type is Common",
                    config.selector.current(),
                    config.path_type.matches(PathType::Common),
                );
                cb.add_constraint(
                    "depth is 0",
                    config.selector.current(),
                    config.depth.current(),
                );
                cb.add_constraint(
                    "direction is 0",
                    config.selector.current(),
                    config.direction.current(),
                );

                // let code_size = (config.old_hash.current() - config.old_value_rlc.current())
                //     * Query::Constant(F::from(1 << 32).invert().unwrap());
                // cb.add_lookup(
                //     "old nonce is 8 bytes",
                //     [config.old_value_rlc.current(), Query::from(7)],
                //     bytes.lookup(),
                // );
                // cb.add_lookup(
                //     "old code size is 8 bytes",
                //     [code_size, Query::from(7)],
                //     bytes.lookup(),
                // );
                // cb.add_lookup(
                //     "hash input is 16 bytes",
                //     [config.old_hash.current(), Query::from(15)],
                //     bytes.lookup(),
                // );
            }
            SegmentType::AccountLeaf4
            | SegmentType::StorageTrie
            | SegmentType::StorageLeaf0
            | SegmentType::StorageLeaf1 => {
                cb.assert_unreachable("asdfasdf", config.selector.current())
            }
        };
        cb.condition(
            config.segment_type.matches(variant),
            conditional_constraints,
        );
    }

    cb.condition(
        config.segment_type.matches(SegmentType::AccountTrie),
        |cb| {
            cb.add_constraint(
                "0",
                config
                    .segment_type
                    .previous_matches(SegmentType::Start)
                    .or(config
                        .segment_type
                        .previous_matches(SegmentType::AccountTrie)),
                Query::one(),
            );
        },
    );
}

fn configure_balance<F: FieldExt>(cb: &mut ConstraintBuilder<F>, config: &MptUpdateConfig) {}

fn configure_code_hash<F: FieldExt>(cb: &mut ConstraintBuilder<F>, config: &MptUpdateConfig) {}

fn configure_empty_account<F: FieldExt>(cb: &mut ConstraintBuilder<F>, config: &MptUpdateConfig) {}

fn configure_self_destruct<F: FieldExt>(cb: &mut ConstraintBuilder<F>, config: &MptUpdateConfig) {}

fn configure_storage<F: FieldExt>(cb: &mut ConstraintBuilder<F>, config: &MptUpdateConfig) {}

fn configure_empty_storage<F: FieldExt>(cb: &mut ConstraintBuilder<F>, config: &MptUpdateConfig) {}

#[cfg(test)]
mod test {
    use super::super::{
        byte_bit::ByteBitGadget, byte_representation::ByteRepresentationConfig,
        canonical_representation::CanonicalRepresentationConfig, key_bit::KeyBitConfig,
        poseidon::PoseidonConfig,
    };
    use super::*;
    use crate::types::{account_key, hash};
    use halo2_proofs::{
        circuit::{Layouter, SimpleFloorPlanner},
        dev::MockProver,
        halo2curves::bn256::Fr,
        plonk::{Circuit, Error},
    };

    #[derive(Clone, Debug)]
    struct TestCircuit {
        updates: Vec<SMTTrace>,
    }

    impl TestCircuit {
        fn hash_traces(&self) -> Vec<(Fr, Fr, Fr)> {
            let mut hash_traces = vec![(Fr::zero(), Fr::zero(), Fr::zero())];
            for update in self.updates.iter() {
                let address_hash_traces = Proof::from(update.clone()).address_hash_traces;
                for (direction, old_hash, new_hash, sibling, is_padding_open, is_padding_close) in
                    &address_hash_traces
                {
                    if *is_padding_open {
                        let (left, right) = if *direction {
                            (sibling, old_hash)
                        } else {
                            (old_hash, sibling)
                        };
                        hash_traces.push((*left, *right, hash(*left, *right)));
                    }
                    if *is_padding_close {
                        let (left, right) = if *direction {
                            (sibling, new_hash)
                        } else {
                            (new_hash, sibling)
                        };
                        hash_traces.push((*left, *right, hash(*left, *right)));
                    }
                }
            }
            hash_traces
        }

        fn keys(&self) -> Vec<Fr> {
            let mut keys = vec![Fr::zero()];
            for update in self.updates.iter() {
                let proof = Proof::from(update.clone());
                let key = account_key(proof.claim.address);
                dbg!(account_key(proof.claim.address));
                keys.push(key);
            }
            dbg!(keys.clone());
            keys
        }

        fn key_bit_lookups(&self) -> Vec<(Fr, usize, bool)> {
            let mut lookups = vec![(Fr::zero(), 0, false)];
            for update in self.updates.iter() {
                let proof = Proof::from(update.clone());
                for (i, (direction, _, _, _, _, _)) in
                    proof.address_hash_traces.iter().rev().enumerate()
                {
                    lookups.push((account_key(proof.claim.address), i, *direction));
                }
            }
            dbg!(lookups.clone());
            lookups
        }
    }

    impl Circuit<Fr> for TestCircuit {
        type Config = (
            // MptUpdateConfig,
            PoseidonConfig,
            CanonicalRepresentationConfig,
            KeyBitConfig,
            ByteBitGadget,
            // ByteRepresentationConfig,
        );
        type FloorPlanner = SimpleFloorPlanner;

        fn without_witnesses(&self) -> Self {
            Self { updates: vec![] }
        }

        fn configure(cs: &mut ConstraintSystem<Fr>) -> Self::Config {
            let mut cb = ConstraintBuilder::new();
            let poseidon = PoseidonConfig::configure(cs, &mut cb);
            let byte_bit = ByteBitGadget::configure(cs, &mut cb);
            // let byte_representation = ByteRepresentationConfig::configure(cs, &mut cb, &byte_bit);
            let canonical_representation =
                CanonicalRepresentationConfig::configure(cs, &mut cb, &byte_bit);
            let key_bit = KeyBitConfig::configure(
                cs,
                &mut cb,
                &canonical_representation,
                &byte_bit,
                &byte_bit,
                &byte_bit,
            );

            // let byte_representation = ByteRepresentationConfig::configure(cs, &mut cb, &byte_bit);

            // let mpt_update = MptUpdateConfig::configure(
            //     cs,
            //     &mut cb,
            //     &poseidon,
            //     &key_bit,
            //     &byte_representation,
            //     &byte_representation,
            // );

            cb.build(cs);
            (
                // mpt_update,
                poseidon,
                canonical_representation,
                key_bit,
                byte_bit,
                // byte_representation,
            )
        }

        fn synthesize(
            &self,
            config: Self::Config,
            mut layouter: impl Layouter<Fr>,
        ) -> Result<(), Error> {
            let (
                // mpt_update,
                poseidon,
                canonical_representation,
                key_bit,
                byte_bit,
                // byte_representation,
            ) = config;

            layouter.assign_region(
                || "asdfasdf",
                |mut region| {
                    // mpt_update.assign(&mut region, &self.updates);
                    poseidon.assign(&mut region, &self.hash_traces());
                    canonical_representation.assign(&mut region, &self.keys());
                    // key_bit.assign(&mut region, &self.key_bit_lookups());
                    byte_bit.assign(&mut region);
                    // byte_representation.assign(&mut region, &self.byte_representations())
                    Ok(())
                },
            )
        }
    }

    #[test]
    fn test_mpt_updates() {
        let circuit = TestCircuit { updates: vec![] };
        let prover = MockProver::<Fr>::run(14, &circuit, vec![]).unwrap();
        assert_eq!(prover.verify(), Ok(()));
    }

    #[test]
    fn test_nonce_updates() {
        const NONCE_TRACES: &str = include_str!("../../tests/nonce.json");
        let updates: Vec<SMTTrace> = serde_json::from_str(NONCE_TRACES).unwrap();

        let circuit = TestCircuit { updates };
        let prover = MockProver::<Fr>::run(14, &circuit, vec![]).unwrap();
        assert_eq!(prover.verify(), Ok(()));
    }
}
