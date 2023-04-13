# Storage Tree Proof

The storage tree proof helps to check updating on accounts and their storage (via SSTORE and SSLOAD op) are correctly integrated into the storage tree, so the change of state root of EVM has represented and only represented the effects from transactions being executed in EVM. The `Account` and `Storage` records in state proof, which would finally take effect on storage tree, need to be picked into storage tree proof. The storage tree circuit provide EVM's state root are updated sequentially and the final root after being updated by the records sequence from state proof coincided with which being purposed in the new block.

## The architecture of state trie

An alternative implement of BMPT (Binary Patricia Merkle Tree) has been applied for the zk-evm as the state trie. The BMPT has replaced the original MPT for world state trie and account storage trie, and a stepwise hashing scheme instead of rlp encoding-hashing has been used for mapping data structures into hashes. For the BMPT implement we have:

+ Replacing all hashing calculations from keccak256 to poseidon hash

+ In BMPT there are only branch and leaf nodes, and their hashes are calculated as following schemes:

    * Branch node is an 2 item node and `NodeHash = H(NodeHashLeft, NodeHashRight)`
    * Leaf node is an 2 item node and `NodeHash = H(H(1, encodedPath), value)`

+ In world state trie, the value of leaf node is obtained from account state and the hashing scheme is: `AccountHash = H(H(nonce, balance), H(H(CodeHash_first16, CodeHash_last16), storageRoot))`, in which `CodeHash_first16` and `CodeHash_last16` represent the first and last 16 bytes of the 32-byte codeHash item

+ In account storage trie, the value of leaf node is obtained from hashing the first and last 16 bytes of the storaged 32-byte value, i.e: `ValueHash = H(value_first16, value_last16)`

## Layouts of the circuit

To show there is value `v` and key `k` existed for account `addr` we need 4 proofs:

1. Proof the stored key and value has been correctly encoded and hashed into the secured key and value in one of the leaf node of account storage trie
2. Proof the BMPT path for the leaf node in proof `1` against the current root `Rs` of storage trie, is correct
3. Proof the account `addr` with stateRoot is `Rs` can be encoded and hashed into one of the leaf node of state trie
4. Proof the BMPT path for the leaf node in proof `3` against the current root `R` of state trie, is correct

To show the root of state trie change from `R0` to `R1` is contributed by updating key `k` for account `addr` from value `v0` to `v1`, we used 4 proofs as described before for providing `(v0, k, addr) -> R0` and another 4 proofs for providing `(v1, k, addr) -> R1`. Then another updating on storage can be applied on the new state trie with root `R1` and transit it to root `R2`. For a series of *n* updates on storage of EVM which transit state trie from root `R0` to `Rn`, our proofs provide the transition via *n* intermediate roots `R1, R2 ... Rn-1`, and provide *n* transitions `R0 -> R1`, `R1 -> R2`, ... , `Rn-1 -> Rn` caused by the *n* updates are all correct.

For the proof of each transition `Ri -> Ri+1` on the trie. Each of the 4 proofs for the start state `Ri` is paired with the 4 proofs for the end state `Ri+1` and the 4 proof pairs are stacked from bottom to top, so the layout would look like:

| state | proof of start | proof of end | trie root |
| ----- | -------------- | ------------ | --------- |
|  ...  |                |              |           |
|   i   |  <proof 4>     |   <proof 4>  |  Ri, Ri+1 |
|       |  <proof 3>     |   <proof 3>  |           |
|       |  <proof 2>     |   <proof 2>  |           |
|       |  <proof 1>     |   <proof 1>  |           |
|  i+1  |  <proof 4>     |   <proof 4>  | Ri+1, Ri+2|

So there are 5 kinds of proof (4 proof 'pair' mentioned before and a 'padding' proof) which need to be layout to the circuit. Columns in circuit are grouped for 3 parts:

+ Controlling part, which enables different proofs being activated in specified row and constrains how adjacent rows for different proofs can transit:

>    * `series` indicate one row is dedicated for the i*th* transition for state trie. The cell of `series` on next row must be the same or only 1 more than the current one.
>    * `selector 0~5` each enable the row for one of the 5 proofs. With constraint `sigma(selector_i) = 1` there would be one and only one selector enabled for each row.
>    * `op_type` can be 0 to 5 and specifies that the row currently works for proof N. And constraint `sigma(selector_i * i) = op_type` binds the value of `op_type` to the enabled `selector_i`.
>    * `op_delta_aux` reflects whether there is a difference between the value of current `op_type` cell and the one above it.
>    * `ctrl_type` is used by different proofs to mark one row for its roles. When the value of `op_type` changed in adjacent rows, only the constrained pairs of `(op_type, ctrl_type)` are allowed so the sequence of proofs stacking is controlled. More specific, we just: 
>        + look up current `(op_type, ctrl_type)` pair from 'external rules' collection when the value of current `op_type` cell is different from the one in above row (the difference must be one)
>        + look up current `(op_type, ctrl_type)` pair from 'internal rules' collection when the value of current `op_delta_aux` is one

+ Data part currently has 3 cols `data_0` ~ `data_2` which dedicate to values whose relations should be provided to be correct by a proof, and 3 additional cols `data_0_ext` ~ `data_2_ext` if 1 field is not enough to represent the value (like codehash and storekey/value). Different proofs assign specified data on those columns: For proofs 1 and 3 (the BMPT proof), the hashes of nodes for the BMPT before and after updating are recorded in `data_0` and `data_1` respectively; for proof 2 `data_0` and `data_1` are used for account hash before and after being updated. Proofs can also refer cells in data columns which belong to the rows adjacent to it, i.e. the data which has been provided by another proof.

There are also 2 limb cols, named as `data_N_limb_0/1`, for each `data_N` col in case when a 256bit variable is splitted and assigned into them as 2 128-bit limbs.

Since the transition is provided in a series of adjacent rows (a "block") in our layout, and the proof of state trie being stacked first. The beginning row of the proof block always contains the start and end trie roots in the transition. So a `root_aux` col is used to 'carry' the end trie root to the last row of the proof block, to ensure the start trie root of next transition must equal to the end trie root of previous proof block. The layout look like follows:

| series| data_0 *for old_root* | data_1 *for new_root* | root_aux |
| ----- | -------- | -------- | -------- |
|  ...  |          |          |          |
|   i   |    Ri    |   Ri+1   |  Ri+1    |
|       |          |          |  Ri+1    |
|       |          |          |  Ri+1    |
|       |          |          |  Ri+1    |
|  i+1  |  Ri+1    |   Ri+2   |  Ri+2    |

The constraint for `root_aux` is:

> `root_aux(cur) = new_root(cur)` if `series` has changed, else `root_aux(cur) = root_aux(prev)`

+ Gadget part has columns dedicated to different proofs. Each kind of proof (BMPT, account hash, value hash or padding) use these columns and custom gates for a proof has to be enabled by the `selector_i` col inside controlling part.

### BMPT transition proof

This provide an updating on the key `k` of BMPT has made its root to change from `Ri` to `Ri+1` under one of the following three possible transitions:

1. A new leaf node with value `v1` is created
2. The leaf node with value `v0` is removed
3. The leaf node with value `v0` is being updated to value `v1`

It is needed to provide the path in BMPT, from root to the leaf node of key `k`, is valid. Both the BMPT path before and after leaf node `k` being updated has to be provided and the two BMPT path shared the same siblings. It take one row to put the data of one layer in the BMPT path, including the type of node (branch or leaf), the hash of node, the prefix bit for the corresponding layer etc. The two BMPT path for providing should has the same depth. In the case of transitions 1 and 2, a non-existing proof, i.e. a BMPT path from root to an empty node should be provided.

For the nature of patricia tree, if there is no leaf with key `k` in the trie and leaf node `k1` which has longest common prefix with `k` in all leafs of the trie. Suppose the length of the common prefix is `l` and currently the length of prefix of leaf node `k1` is still less than `l`, then the depth of BMPT path would be changed after being updated. In this case, in the (non-existing) proof for the empty node of key `k`, the BMPT path has to be re-organized to reflect the trie state right before leaf node of key `k` being updated to an empty node, or right after the leaf node being removed and the empty node left. Take the following as an example:

![a Merkle tree storing example](https://i.imgur.com/SaLpIn3.png)

+ While only leaf node A and B is inserted, the prefix path for node B (key 1000) is 1, and the root of current trie is `Rb`;
+ New node C (key 1010) will be inserted. For proof, we use the re-organized trie state in which leaf node C just updates the empty node with key 1010, and the prefix path of this empty node is 101.
+ For such a situation, the prefix path for node B has become 100 instead of 1.
+ Notice this is a 'virtual' state for the trie, for the root of trie doesn't change from `Rb`. To provide this virtual BMPT path, we induce a special node types for the reorganized branch node (whose prefix path is 1 and 10 in our case).

Since we are using binary Merkle tree, each layer in the tree would take one bit from the key. From top (root) of the trie, the least-significant bit in the key would be checked and for the leaf node with `l` bits as prefix, the partial key for leaf part would just right shift the original key by `l`. For example, for a leaf node with key is 19 (`B10011`) with 3 bits prefix for path, the path (from root to leaf) is `1-1-0` and the partial key is 2 (`B10`).

Following is the layout of a BMPT path in proof, one row for each layer:

![layout of a BMPT path](https://gist.githubusercontent.com/noel2004/40cc19fa97924d0e383ef6b7e53d5e6d/raw/23797d692ef91cd47d34fb4942a9a38c50de959c/1.svg)

The BMPT transition proof uses the following columns:

> `Old/NewHashType`: Record the type of a node in current row, the two columns `Old-` and `New-` is dedicated to the state of trie before and after updating respectively. There are 6 types would be used:
>  + `Start`: indicate the node is dedicated for the root hash of trie, both in old- and new- state the rows for BMPT path should start with this node
>  + `Mid`: indicate a branch node
>  + `Leaf`: indicate a leaf node
>  + `Empty`: indicate an empty node, the hash of this node is Fq::Zero
>  + `LeafExt`: indicate a "virtual" node in the leaf inserted / deleted case, which in fact exist only in the updated state, the node hash for these node types is just equal to its child
>  + `LeafExtFinal`: Parent of the empty node which would be updated

> `sibling`: The siblings of nodes in BMPT path

> `path`: The prefix bit in each layer of BMPT path, for the last layer (which the leaf node lies), record the residue of current key

> `depth`: An aux column which start from 1 on the first row of proof and double in the cell of next row

and defining following columns in the data part:

> `Old/NewVal` for `data_0` and `data_1`: For branch node, record the hash of its child which is in the BMPT path; for leaf node, record the value

> `accKey` for `data_2`: Calculate the whole key from `path` and `depth` cols, as shown in the "key" column in the diagram before, so the output is laid in the cell of last row of the proof

**Constraints**

There are two groups of constraints: one for the validity of BMPT path and one for the validity of the correction in state transition

There are two BMPT paths in the proof (for the columns with 'Old-' and 'New-' prefix), to provide the validity of them we should:

+ construct and look up the hash calculations from hash table (see below) according to the node type for current row:

    * For branch node (type is `Mid`), the hashing input has two fields which are from `-Val` and `Sibling`, and the output in `-Val(prev)` (cell above current row). They are lookup according to current `path` cell: when `path` is 1, lookup `Poseidon(Sibling, -Val) = -Val(prev)`, else lookup `Poseidon(-Val, Sibling) = -Val(prev)`
    * For leaf node (type is `Leaf`) we lookup for the leaf scheme: `Poseidon(1, accKey, -Val) = -Val(prev)`
    * For node type `LeafExt`, we constraint current `-Val` cell is equal to its child, i.e `-Val(next)` (cell below current row)
    * For node type `LeafExtFinal`, we constraint current `sibling` cell is equal to the `-Val(prev)`

+ calculate the key in `accKey` col and make output in the bottom layer:

    * constraint cell in `depth` is double to the cell above of it: `depth = 2*depth(prev)` and the first cell in `depth` is `Fq::one`
    * constraint cell in `accKey` in the recursive way: `accKey = depth(prev) * path + accKey(prev)` and the first cell in `accKey` is zero

+ and also ensure the layout of path is valid, i.e: for a specified type in `-hashType`, there is only limited possible value for the cell below it, we lookup `-hashType, -hashType(prev)` from following constant transition rules:

>    * `Start -> Mid / Leaf / Empty / LeafExt / LeafExtFinal`
>    * `Mid -> Mid / Empty / Leaf / LeafExt / LeafExtFinal`
>    * `LeafExt -> LeafExt / LeafExtFinal`
>    * `LeafExtFinal -> Empty`

+ finally, we must constraint a proof must end its layout with `Empty` or `Leaf` node. We just assign `OldHashType` as the `ctrl_type` column in controlling part. For `op_type` is 1 or 3 (the two BMPT proof for two tries), the value of `op_type` can be change only when `ctrl_type` is `Empty` or `Leaf`

To provide the transition, i.e. the relationship between two BMPT path is correct, we constraint the two node type in the same layer, that is, `OldHashType` and `NewHashType` has to be one of the following pairs (Notice the reversed of one pair is also valid):

>    * `Start` - `Start`
>    * `Leaf` - `Leaf`
>    * `Empty` - `Leaf`
>    * `Mid` - `Mid`
>    * `LeafExt` - `Mid`
>    * `LeafExtFinal` - `Mid`


### Account data proof

This proves that the account data is consistant with its hash under the new zktrie scheme (not the original RLP encoding scheme) and the hash act as the value of leaf node in state trie.

There are a pair of proofs which validate the account data before and after updating,  respectively. For each proof we use the following columns:

> + `data_0` and `data_1` in the data part contain all fields in account data except for the account root (the root of storage trie for current account), i.e. the `nonce`, `balance` and `code_hash`, for the 32 bytes codeHash, two cols would be assigned for the two 16 bytes limbs (first and last 16 bytes) and the RLC of them is recorded in `data_0/1` col.
> + For codehash, which require 2 field to be represented, the `data_0/1_ext` are also used
> + `Intermediate_1`, `Intermediate_2`: contain account root and some intermediate value being use in the hashing scheme

The layout for account proof looks like following:

|op_type|ctrl_type| Intermediate_1  | Intermediate_2  |    data_0/1    |  data_0/1_ext |
|-------|---------|-----------------|-----------------|----------------|---------------|
|   1   |         |                 |                 |   *hash_final* |               |
|   2   |    0    |                 |   hash_final    |    nonce       |               |
|   2   |    1    |      hash3      |      hash2      |    balance     |               |
|   2   |    2    |      hash1      |      Root       |  code_hash_hi  | code_hash_low |
|   3   |         |                 |                 |      *Root*    |               |

value of `1`, `2`, `3` in `op_type` col indicate the row dedicating to proof of state trie (proof 4), account data (proof 3) and storage trie (proof 2) respectively. we can see in account data proof  the top and bottom cell in data columns can be easily constrained to be equal to the cell above / below them. So the proofs are being "connected".

The proof also lookup hashes for the hashing scheme:

>    * `Poseidon(Codehash_hi, Codehash_low) = hash1`
>    * `Poseidon(nonce, balance) = hash3`
>    * `Poseidon(hash1, Root) = hash2`
>    * `Poseidon(hash3, hash2) = hash_final`

**Constraints**

Currently the proof has a layout with 3 or 4 rows, with cells in `ctrl_type` col being assigned from 0 ~ 2 (or 3, in case of 4 rows). And it also has a gate for some equality:

>    * ctrl_type is 0: `hash_final = data_0/1 (prev)`
>    * ctrl_type is 2: `Root = data_0/1 (next)`

The 4 rows layout is used when the values in `OldHash` and `NewHash` are equal. The value of 3 in `ctrl_type` indicate the row below current proof is not dedicated for a BMPT proof (proof 2) but a proof block for another state transition, since more proof is not needed when the storage roof of current account is unchanged.

### Storage value proof

This provide the hash of stored value and key is consistent with the key / value of leaf node in the BMPT proof for storage proof (proof 2). It has a one-row layout as follows:

|op_type|ctrl_type|    data_0     |   data_0_ext   |    data_1     |   data_1_ext   |   data_2  | data_2_ext |
|-------|---------|---------------|----------------|---------------|----------------|-----------|------------|
|   3   |         |   *s_hash*    |                |   *e_hash*    |                |*key_hash* |            |
|   4   |   0     | s_value_first | s_value_second | e_value_first | e_value_second | key_first | key_second |

The data cols is assigned with the RLC of their corresponding 16-byte limbs and hash is being lookup:

>    * `Poseidon(s_value_first, s_value_second) = s_hash`
>    * `Poseidon(e_value_first, e_value_second) = e_hash`
>    * `Poseidon(key_first, key_second) = key_hash`

The value in `ctrl_type` is fixed to 0

### padding 

 Padding proof is used to fill the unused rows, it just constrain the `data_0` and `data_1` should be equal to each other, and cell in `data_1` col must equal to which above it. So the last cell in `data_1` col in mpt circuit would always equal to the last hash provided by a proof except padding.

## Hash table

Proved by poseidon hash circuit. Inputs and output for each hash calculation are put in the same row. For a hash circuit which calculate the hash of at most N items we have N+2 cols: 

>  - **Items** the number of items in the calculation
>  - **1..N (Fields)** N cols for at most N items
>  - **Hash** then the col for the hash.

Currently the least N we need is 3:

| 0 Items| 1  | 2  | 3  | Hash       |
| ---    |--- |--- |--- | ---        |
|   1    | FQ1|    |    | FQ         |
|   3    | FQ1| FQ2| FQ3| FQ         |
|   3    | FQ1| FQ2| FQ3| FQ         |

## MPT table

