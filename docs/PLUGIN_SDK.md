# Plugin SDK contract (draft)

The `kordra-plugin-sdk` crate defines a versioned serialized manifest with capability types: download engine, extractor, media processor, automation, UI widget, notifications and cloud provider.

Permission vocabulary covers network hosts, filesystem read/write roots, secret keys and event names.

**Important:** this is a schema design, not a working plugin loader or sandbox. Kordra must never execute untrusted plugin code until permission enforcement, signed packages and process/WASM isolation have been implemented.
