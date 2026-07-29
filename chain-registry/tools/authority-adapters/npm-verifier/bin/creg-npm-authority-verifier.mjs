#!/usr/bin/node

import { dirname } from "node:path";
import { fileURLToPath } from "node:url";

import { main } from "../src/sidecar.mjs";

await main(dirname(fileURLToPath(import.meta.url)));
