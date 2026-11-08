# CyberChef tooling

Used to port CyberChef operations faithfully. Neither script ships in the app.

## Test vectors

`export.mjs` loads every file in CyberChef's `tests/operations/tests` with a
stub `TestRegister` (`hooks.mjs`, `stub-register.mjs`) and writes the vectors
as JSON. It needs no npm install:

```sh
node scripts/cyberchef/export.mjs /path/to/CyberChef/tests/operations/tests \
  workbench/utilify-chef/tests/vectors/cyberchef.json
```

## Oracle

`oracle.mjs` bakes recipes with real CyberChef, for checking cases the vectors
don't cover. It runs from inside a CyberChef checkout with its dependencies:

```sh
cp -R /path/to/CyberChef /tmp/cyberchef-run && cd /tmp/cyberchef-run
npm ci --ignore-scripts
node src/core/config/scripts/fixCryptoApiImports.mjs
echo '[]' > src/core/config/OperationConfig.json
for s in generateHTMLEntities generateOpsIndex generateConfig; do
  node --no-warnings --openssl-legacy-provider src/core/config/scripts/$s.mjs
done
cp /path/to/utilify/scripts/cyberchef/oracle.mjs .
node --no-warnings --no-deprecation --openssl-legacy-provider oracle.mjs cases.json
```

`cases.json` is `[{"input": "...", "recipe": [{"op": "To Hex", "args": ["Space", 0]}]}]`;
the result is `[{"output": "..."} | {"error": "..."} | {"crash": "..."}]`.
