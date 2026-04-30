-- Views that expose `resolved_channel` — the friendly channel name from
-- v1.registries when available, otherwise the raw emitter contract_id.
--
-- The v1 pipeline writes emitter contract_ids into the `channel` column
-- of v1.published_wasms and v1.registered_contracts (the `contract_id as
-- channel` projection in goldsky/v1/index.yaml). Callers of the API
-- pass friendly names like "root" or "soroswap", so every read needs to
-- translate contract_id → name via v1.registries. These views
-- encapsulate that translation so individual queries don't each repeat
-- the LEFT JOIN + COALESCE.

CREATE OR REPLACE VIEW v1.published_wasms_with_channel AS
SELECT
  w.*,
  r.registry_channel AS channel
FROM v1.published_wasms w
JOIN v1.registries r ON r.contract_id = w.emitter_contract_id;

CREATE OR REPLACE VIEW v1.registered_contracts_with_channel AS
SELECT
  c.*,
  r.registry_channel AS channel
FROM v1.registered_contracts c
JOIN v1.registries r ON r.contract_id = c.emitter_contract_id;

-- StrKey contract encoding: 32-byte hex hash → 56-char "C…" form.
--
-- archive.deploys.new_contract_hash is the raw 32-byte hash returned by
-- operation_result.invoke_host_function.success, encoded as lowercase
-- hex. Every other contract_id in the schema (registered_contracts,
-- deployed_contracts, archive.upgrades.upgraded_contract_id) is the
-- StrKey C… form decoded from event payloads. To filter v1.versions by
-- a caller-supplied StrKey contract_id we need an in-database encoder
-- — Flink has no UDF for this, so the bridge has to live in Postgres.
--
-- Encoding follows SEP-23:
--   1. payload = 0x10 (contract version byte: 2 << 3) || 32-byte hash  (33 bytes)
--   2. crc    = CRC16-XMODEM(payload)
--   3. result = base32_no_padding(payload || lo(crc) || hi(crc))       (35 → 56 chars)

CREATE OR REPLACE FUNCTION v1.crc16_xmodem(input bytea)
RETURNS integer
LANGUAGE plpgsql IMMUTABLE STRICT AS $$
DECLARE
  crc integer := 0;
  i   integer;
  j   integer;
BEGIN
  FOR i IN 0..octet_length(input) - 1 LOOP
    crc := (crc # (get_byte(input, i) << 8)) & 65535;
    FOR j IN 1..8 LOOP
      IF (crc & 32768) <> 0 THEN
        crc := ((crc << 1) # 4129) & 65535;
      ELSE
        crc := (crc << 1) & 65535;
      END IF;
    END LOOP;
  END LOOP;
  RETURN crc;
END;
$$;

CREATE OR REPLACE FUNCTION v1.base32_encode(input bytea)
RETURNS text
LANGUAGE plpgsql IMMUTABLE STRICT AS $$
DECLARE
  alphabet constant text := 'ABCDEFGHIJKLMNOPQRSTUVWXYZ234567';
  out_text text := '';
  bits     int  := 0;
  buf      int  := 0;
  i        int;
BEGIN
  FOR i IN 0..octet_length(input) - 1 LOOP
    buf  := (buf << 8) | get_byte(input, i);
    bits := bits + 8;
    WHILE bits >= 5 LOOP
      bits := bits - 5;
      out_text := out_text || substr(alphabet, ((buf >> bits) & 31) + 1, 1);
    END LOOP;
  END LOOP;
  IF bits > 0 THEN
    out_text := out_text || substr(alphabet, ((buf << (5 - bits)) & 31) + 1, 1);
  END IF;
  RETURN out_text;
END;
$$;

CREATE OR REPLACE FUNCTION v1.strkey_contract(hex_hash text)
RETURNS text
LANGUAGE plpgsql IMMUTABLE AS $$
DECLARE
  versioned bytea;
  crc       int;
BEGIN
  IF hex_hash IS NULL OR length(hex_hash) <> 64 THEN
    RETURN NULL;
  END IF;
  versioned := decode('10', 'hex') || decode(hex_hash, 'hex');
  crc := v1.crc16_xmodem(versioned);
  RETURN v1.base32_encode(
    versioned
    || decode(lpad(to_hex(crc & 255),        2, '0'), 'hex')
    || decode(lpad(to_hex((crc >> 8) & 255), 2, '0'), 'hex')
  );
END;
$$;

-- v1.wasm_versions — one row per (uploaded wasm × publish event), with
-- bare uploads kept as rows whose publish-side columns are NULL. This
-- view used to be called v1.versions; it was renamed when v1.versions
-- was repurposed to mean "the version history of a contract".
--
-- archive.uploads is the canonical, hash-addressable inventory of wasm
-- bytecode. v1.published_wasms records `publish` events emitted by
-- registry contracts, decorating a wasm_hash with author + name +
-- semver. The same wasm_hash can be published into many registries
-- (one row per channel), and a wasm can be uploaded without ever being
-- published — a LEFT JOIN from uploads to published_wasms keeps both
-- shapes in one view.

DROP VIEW IF EXISTS v1.versions;  -- old definition (upload × publish join)

CREATE OR REPLACE VIEW v1.wasm_versions AS
SELECT
  u.wasm_hash,
  u.id                         AS upload_id,
  u.ledger_sequence            AS uploaded_ledger_sequence,
  u.closed_at                  AS uploaded_at,
  u.transaction_hash           AS upload_transaction_hash,
  u.source_account             AS uploader,
  p.id                         AS publish_id,
  p.transaction_hash           AS publish_transaction_hash,
  p.ledger_sequence            AS publish_ledger_sequence,
  p.created_at                 AS published_at,
  p.author,
  p.wasm_name,
  p.wasm_version,
  p.emitter_contract_id        AS publish_registry_contract_id
FROM archive.uploads u
LEFT JOIN v1.published_wasms p ON p.wasm_hash = u.wasm_hash;

-- v1.versions — version history of a contract.
--
-- One row per (contract_id × wasm transition), ordered chronologically
-- within each contract. The first row (version_index = 0, kind =
-- 'initial') is the wasm the contract was deployed with; subsequent
-- rows (kind = 'upgrade') are runtime executable_update events.
--
-- Sources:
--   archive.deploys     — host-function CreateContract / CreateContractV2.
--                         Anchors the initial wasm. Keyed by hex
--                         contract hash, encoded to StrKey via
--                         v1.strkey_contract(). SAC creates (NULL
--                         wasm_hash) are excluded.
--   archive.upgrades    — system `executable_update` events. Already
--                         keyed by StrKey contract_id, no encoding
--                         needed.
--
-- Each row carries transition provenance only — kind, source_id (id
-- from the underlying archive table), ledger_sequence,
-- transaction_hash, created_at, and the wasm_hash that became active
-- at that transition. Publish metadata (author / wasm_name /
-- wasm_version) is intentionally not joined here; callers that need
-- it can JOIN v1.published_wasms ON wasm_hash themselves, or use
-- v1.wasm_versions for the upload-side view.
--
-- Typical use:
--   SELECT * FROM v1.versions
--   WHERE contract_id = 'C…'
--   ORDER BY version_index;

CREATE VIEW v1.versions AS
SELECT
  contract_id,
  ROW_NUMBER() OVER (
    PARTITION BY contract_id
    ORDER BY ledger_sequence, source_id
  ) - 1 AS version_index,
  kind,
  wasm_hash,
  source_id,
  ledger_sequence,
  transaction_hash,
  created_at
FROM (
  SELECT
    v1.strkey_contract(d.new_contract_hash) AS contract_id,
    'initial'::text                          AS kind,
    d.wasm_hash                              AS wasm_hash,
    d.id                                     AS source_id,
    d.ledger_sequence                        AS ledger_sequence,
    d.transaction_hash                       AS transaction_hash,
    d.closed_at                              AS created_at
  FROM archive.deploys d
  WHERE d.new_contract_hash IS NOT NULL
    AND d.wasm_hash IS NOT NULL
  UNION ALL
  SELECT
    u.upgraded_contract_id,
    'upgrade'::text,
    u.new_wasm_hash,
    u.id,
    u.ledger_sequence,
    u.transaction_hash,
    u.created_at
  FROM archive.upgrades u
) transitions;
