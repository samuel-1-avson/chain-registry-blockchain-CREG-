import { dirname } from "node:path";

import { main } from "./sidecar.mjs";

void main(dirname(process.execPath));
