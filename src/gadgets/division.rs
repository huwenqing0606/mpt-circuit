use halo2_proofs::{
    arithmetic::{Field, FieldExt},
    circuit::{Chip, Layouter, Region, Value},
    plonk::{
        Advice, Column, ConstraintSystem, Error, Expression, Selector, TableColumn, VirtualCells,
    },
    poly::Rotation,
};

// The field modulus for Fr 2^ 253 < P < 2^254
// Proves that dividend = 2^248 * quotient + remainder, with the following constraints:
//   1) dividend = 2^248 * quotient + remainder mod P
//   2) 0 <= remainder < 2^248
//   3) 0 <= quotient  <= 2^5
// there will be quotients that are between 2^5 and 2^6 though?
// what are the leading bits of the field modulus? it's fine if it's 0xb100000000.... with enough leading 0's.

// 1) implies that dividend = 2^248 * quotient + remainder + n * P for some integer n.
// we want show to n is 0...

// n * P = dividend - 2^248 * quotient + remainder
// 0 <= n * P < 2^254 + 2^254 = 2^255 so n is 0 or 1.... but we need it to be 0
// what if you
//                        P < 2^256 +
// 0 <= dividend < P < 2^256, so there is exactly 1 quotient

struct DivisionConfig {
    dividend: Column<Advice>,
    quotient: Column<Advice>,
    remainder: [Column<Advice>; 31],
}

// impl DivisionConfig {
//     fn configure<F: Field>(meta: &mut ConstraintSystem<F>) -> Self {
//         Self(meta.fixed_column())
//     }

//     fn assign<F: Field>(&self, layouter: &mut impl Layouter<F>) -> Result<(), Error> {
//         layouter.assign_region(
//             || "byte range check fixed column",
//             |mut region| {
//                 (0..256)
//                     .map(|i| region.assign_advice(|| "", self.0, i, || i.into()))
//                     .collect()
//             },
//         )
//     }

//     pub(crate) fn lookup_expressions<F: Field>(
//         &self,
//         meta: &mut VirtualCells<'_, F>,
//     ) -> Vec<Expression<F>> {
//         vec![meta.query_fixed(self.0, Rotation::cur())]
//     }
// }


#[cfg(test)]
mod test {
    use super::*;
    use halo2_proofs::{
        circuit::SimpleFloorPlanner, dev::MockProver, halo2curves::bn256::Fr, plonk::Circuit,
    };

    #[test]
    fn circuit() {
        dbg!(Fr::modulus);
        panic!();
    }
}
