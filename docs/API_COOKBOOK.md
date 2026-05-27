# API Cookbook

Base URL: `http://localhost:8080` (default node REST).

## Public reads

```bash
curl -s http://localhost:8080/v1/public/health | jq
curl -s http://localhost:8080/v1/public/chain/stats | jq
curl -s 'http://localhost:8080/v1/public/packages?limit=10' | jq
```

## Publisher submit

```bash
# Requires signed PublishRequest body — use `creg publish` or see OpenAPI
curl -s http://localhost:8080/v1/openapi.json
```

## Validator vote

```bash
curl -s -X POST http://localhost:8080/v1/validator/consensus/vote \
  -H 'Content-Type: application/json' \
  -d @vote.json
```

## Relayer (separate service :8083)

```bash
curl -s http://localhost:8083/v1/relayer/policy | jq
curl -s -X POST http://localhost:8083/v1/relayer/quote -H 'Content-Type: application/json' -d @quote.json
curl -s -X POST http://localhost:8083/v1/relayer/sponsor -H 'Content-Type: application/json' -d @sponsor.json
curl -s http://localhost:8083/v1/relayer/status/<request_id> | jq
```

## Operator (requires API key)

```bash
curl -s http://localhost:8080/v1/operator/runtime/config \
  -H "X-Operator-Key: $CREG_OPERATOR_API_KEY"
```

## JSON-RPC

```bash
curl -s http://localhost:8080/rpc \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"creg_health","params":[]}'
```
