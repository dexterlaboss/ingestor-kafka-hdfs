export SVC_CONFIG_PATH="docker/config/.env.test"
echo "Starting ingestor-kafka-hbase with config from $SVC_CONFIG_PATH"

RUST_LOG=info ./target/release/ingestor-kafka-hbase \
  --disable-blocks \
  --disable-tx-compression \
  --disable-tx-by-addr-compression \
  --filter-tx-voting \
  --filter-tx-by-addr-voting \
  --filter-tx-by-addr-exclude-addr=TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA \
  --filter-tx-by-addr-exclude-addr=11111111111111111111111111111111 \
  --filter-tx-by-addr-exclude-addr=ComputeBudget111111111111111111111111111111 \
  --filter-tx-by-addr-exclude-addr=SW1TCH7qEPTdLsDHRgPuMQjbQxKdH2aBStViMFnt64f \
  --filter-tx-by-addr-exclude-addr=Vote111111111111111111111111111111111111111 \
  --filter-tx-by-addr-exclude-addr=So11111111111111111111111111111111111111112 \
  --filter-tx-by-addr-exclude-addr=ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL \
  --filter-tx-by-addr-exclude-addr=MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr \
  --filter-tx-by-addr-exclude-addr=pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA \
  --filter-tx-by-addr-exclude-addr=JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4 \
  --filter-tx-by-addr-exclude-addr=CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C \
  --filter-tx-by-addr-exclude-addr=LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo \
  --filter-tx-by-addr-exclude-addr=GS4CU59F31iL7aR2Q8zVS8DRrcRnXX1yjQ66TqNVQnaR \
  --filter-tx-by-addr-exclude-addr=D1ZN9Wj1fRSUQfCjhvnu1hqDMT7hzjzBBpi12nVniYD6 \
  --filter-tx-by-addr-exclude-addr=ADyA8hdefvWN2dbGGWFotbzWxrAvLW83WG6QCVXvJKqw