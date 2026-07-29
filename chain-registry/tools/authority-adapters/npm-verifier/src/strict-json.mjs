import { reject } from "./errors.mjs";

const decoder = new TextDecoder("utf-8", { fatal: true });
const NUMBER = /^-?(?:0|[1-9][0-9]*)(?:\.[0-9]+)?(?:[eE][+-]?[0-9]+)?/;

function hasLoneSurrogate(value) {
  for (let index = 0; index < value.length; index += 1) {
    const code = value.charCodeAt(index);
    if (code >= 0xd800 && code <= 0xdbff) {
      const next = value.charCodeAt(index + 1);
      if (!(next >= 0xdc00 && next <= 0xdfff)) {
        return true;
      }
      index += 1;
    } else if (code >= 0xdc00 && code <= 0xdfff) {
      return true;
    }
  }
  return false;
}

class Parser {
  constructor(text, label, maxDepth) {
    this.text = text;
    this.label = label;
    this.maxDepth = maxDepth;
    this.offset = 0;
  }

  fail(message) {
    reject(`${this.label} is not strict JSON: ${message}`);
  }

  skipWhitespace() {
    while (
      this.offset < this.text.length &&
      (this.text[this.offset] === " " ||
        this.text[this.offset] === "\n" ||
        this.text[this.offset] === "\r" ||
        this.text[this.offset] === "\t")
    ) {
      this.offset += 1;
    }
  }

  parse() {
    this.skipWhitespace();
    const value = this.parseValue(0);
    this.skipWhitespace();
    if (this.offset !== this.text.length) {
      this.fail("trailing data");
    }
    return value;
  }

  parseValue(depth) {
    if (depth > this.maxDepth) {
      this.fail("nesting exceeds the limit");
    }
    const character = this.text[this.offset];
    if (character === "{") {
      return this.parseObject(depth);
    }
    if (character === "[") {
      return this.parseArray(depth);
    }
    if (character === '"') {
      return this.parseString();
    }
    if (character === "t" && this.text.startsWith("true", this.offset)) {
      this.offset += 4;
      return true;
    }
    if (character === "f" && this.text.startsWith("false", this.offset)) {
      this.offset += 5;
      return false;
    }
    if (character === "n" && this.text.startsWith("null", this.offset)) {
      this.offset += 4;
      return null;
    }
    if (character === "-" || (character >= "0" && character <= "9")) {
      return this.parseNumber();
    }
    this.fail("unexpected token");
  }

  parseObject(depth) {
    this.offset += 1;
    const result = Object.create(null);
    this.skipWhitespace();
    if (this.text[this.offset] === "}") {
      this.offset += 1;
      return result;
    }
    while (this.offset < this.text.length) {
      if (this.text[this.offset] !== '"') {
        this.fail("object member name is not a string");
      }
      const key = this.parseString();
      if (Object.hasOwn(result, key)) {
        this.fail("duplicate object member");
      }
      this.skipWhitespace();
      if (this.text[this.offset] !== ":") {
        this.fail("object member has no colon");
      }
      this.offset += 1;
      this.skipWhitespace();
      result[key] = this.parseValue(depth + 1);
      this.skipWhitespace();
      if (this.text[this.offset] === "}") {
        this.offset += 1;
        return result;
      }
      if (this.text[this.offset] !== ",") {
        this.fail("object members are not comma-separated");
      }
      this.offset += 1;
      this.skipWhitespace();
    }
    this.fail("unterminated object");
  }

  parseArray(depth) {
    this.offset += 1;
    const result = [];
    this.skipWhitespace();
    if (this.text[this.offset] === "]") {
      this.offset += 1;
      return result;
    }
    while (this.offset < this.text.length) {
      result.push(this.parseValue(depth + 1));
      this.skipWhitespace();
      if (this.text[this.offset] === "]") {
        this.offset += 1;
        return result;
      }
      if (this.text[this.offset] !== ",") {
        this.fail("array elements are not comma-separated");
      }
      this.offset += 1;
      this.skipWhitespace();
    }
    this.fail("unterminated array");
  }

  parseString() {
    const start = this.offset;
    this.offset += 1;
    let escaped = false;
    while (this.offset < this.text.length) {
      const code = this.text.charCodeAt(this.offset);
      if (!escaped && code === 0x22) {
        this.offset += 1;
        let value;
        try {
          value = JSON.parse(this.text.slice(start, this.offset));
        } catch (error) {
          this.fail(`invalid string (${error.message})`);
        }
        if (hasLoneSurrogate(value)) {
          this.fail("string contains an unpaired surrogate");
        }
        return value;
      }
      if (!escaped && code < 0x20) {
        this.fail("string contains an unescaped control character");
      }
      if (!escaped && code === 0x5c) {
        escaped = true;
      } else {
        escaped = false;
      }
      this.offset += 1;
    }
    this.fail("unterminated string");
  }

  parseNumber() {
    const match = NUMBER.exec(this.text.slice(this.offset));
    if (!match) {
      this.fail("invalid number");
    }
    this.offset += match[0].length;
    const value = Number(match[0]);
    if (!Number.isFinite(value)) {
      this.fail("number is not finite");
    }
    return value;
  }
}

export function parseJsonStrict(raw, label, { maxBytes, maxDepth = 64 } = {}) {
  if (!Buffer.isBuffer(raw) && !(raw instanceof Uint8Array)) {
    reject(`${label} must be bytes`);
  }
  if (maxBytes !== undefined && raw.byteLength > maxBytes) {
    reject(`${label} exceeds its byte limit`);
  }
  let text;
  try {
    text = decoder.decode(raw);
  } catch (error) {
    reject(`${label} is not strict UTF-8 JSON`, undefined, { cause: error });
  }
  if (text.length === 0) {
    reject(`${label} is empty`);
  }
  return new Parser(text, label, maxDepth).parse();
}

function canonicalizeValue(value, stack) {
  if (value === null || typeof value === "boolean") {
    return JSON.stringify(value);
  }
  if (typeof value === "number") {
    if (!Number.isFinite(value)) {
      reject("JSON value is not representable by RFC 8785");
    }
    return JSON.stringify(value);
  }
  if (typeof value === "string") {
    if (hasLoneSurrogate(value)) {
      reject("JSON value contains an unpaired surrogate");
    }
    return JSON.stringify(value);
  }
  if (typeof value !== "object") {
    reject("JSON value has an unsupported type");
  }
  if (stack.has(value)) {
    reject("JSON value contains a cycle");
  }
  stack.add(value);
  let encoded;
  if (Array.isArray(value)) {
    encoded = `[${value.map((item) => canonicalizeValue(item, stack)).join(",")}]`;
  } else {
    const members = Object.keys(value)
      .sort()
      .map((key) => {
        if (hasLoneSurrogate(key)) {
          reject("JSON object name contains an unpaired surrogate");
        }
        return `${JSON.stringify(key)}:${canonicalizeValue(value[key], stack)}`;
      });
    encoded = `{${members.join(",")}}`;
  }
  stack.delete(value);
  return encoded;
}

// RFC 8785 delegates primitive serialization and UTF-16 member ordering to
// ECMAScript. Inputs used by this verifier contain only I-JSON-safe numbers.
export function canonicalize(value) {
  return Buffer.from(canonicalizeValue(value, new Set()), "utf8");
}
