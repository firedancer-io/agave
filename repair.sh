rm -f /data/chali/agave-testnet.log && \
rm -rf /data/chali/agave/testnet/ledger && \
mkdir /data/chali/agave/testnet/ledger && \
cp /data/chali/agave/testnet/snapshot-* /data/chali/agave/testnet/ledger/ && \
cp /data/chali/agave/testnet/incremental-snapshot-* /data/chali/agave/testnet/ledger/ && \
./cargo run -r --bin agave-validator -- \
    --no-voting \
    --identity ~/.firedancer/fd1/testnet-id-3.json \
    --ledger /data/chali/agave/testnet/ledger \
    --rpc-port 8899 \
    --entrypoint 35.214.172.227:8001 \
    --dynamic-port-range 8000-8079 \
    --log /data/chali/agave-testnet.log \
    --no-os-network-limits-test \
    --maximum-local-snapshot-age 1000000 \
    --no-incremental-snapshots \
    --snapshot-interval-slots 1000000 \
    --no-snapshot-fetch \
    --repair-slot 368263395

    16:11:07
    16:13:37
    RepairService: repaired 1000 slots in Ok(150.648276311s). start: 368262395. end: 368263395

    RepairService: repaired 1000 slots in Ok(49.811254488s). start: 368262395. end: 368263395" location="core/src/repair/repair_service.rs:797:25" version="3.0.8 (src:b4d1c774; feat:3604001754, client:Agave)"
