// TopClaw Arduino Uno firmware.
//
// Protocol: newline-delimited JSON.
// Request:  {"id":"1","cmd":"gpio_write","args":{"pin":13,"value":1}}
// Response: {"id":"1","ok":true,"result":"done"}

const int kLedPin = 13;
String g_input;

String jsonEscape(const String& value) {
  String escaped;
  escaped.reserve(value.length() + 8);
  for (unsigned int i = 0; i < value.length(); ++i) {
    const char c = value.charAt(i);
    if (c == '"' || c == '\\') {
      escaped += '\\';
      escaped += c;
    } else if (c == '\n') {
      escaped += "\\n";
    } else if (c == '\r') {
      escaped += "\\r";
    } else {
      escaped += c;
    }
  }
  return escaped;
}

String extractStringField(const String& payload, const String& key) {
  const String needle = "\"" + key + "\":\"";
  const int start = payload.indexOf(needle);
  if (start < 0) {
    return "";
  }

  const int value_start = start + needle.length();
  const int value_end = payload.indexOf('"', value_start);
  if (value_end < 0) {
    return "";
  }
  return payload.substring(value_start, value_end);
}

long extractIntField(const String& payload, const String& key, long fallback) {
  const String needle = "\"" + key + "\":";
  const int start = payload.indexOf(needle);
  if (start < 0) {
    return fallback;
  }

  int value_start = start + needle.length();
  while (value_start < payload.length() && payload.charAt(value_start) == ' ') {
    ++value_start;
  }

  int value_end = value_start;
  while (value_end < payload.length()) {
    const char c = payload.charAt(value_end);
    if ((c >= '0' && c <= '9') || c == '-') {
      ++value_end;
    } else {
      break;
    }
  }

  if (value_end <= value_start) {
    return fallback;
  }
  return payload.substring(value_start, value_end).toInt();
}

void sendOkString(const String& id, const String& result) {
  Serial.print("{\"id\":\"");
  Serial.print(id);
  Serial.print("\",\"ok\":true,\"result\":\"");
  Serial.print(jsonEscape(result));
  Serial.println("\"}");
}

void sendOkJson(const String& id, const String& result_json) {
  Serial.print("{\"id\":\"");
  Serial.print(id);
  Serial.print("\",\"ok\":true,\"result\":");
  Serial.print(result_json);
  Serial.println("}");
}

void sendError(const String& id, const String& error) {
  Serial.print("{\"id\":\"");
  Serial.print(id);
  Serial.print("\",\"ok\":false,\"result\":\"\",\"error\":\"");
  Serial.print(jsonEscape(error));
  Serial.println("\"}");
}

void handleRequest(const String& payload) {
  String id = extractStringField(payload, "id");
  if (id.length() == 0) {
    id = "0";
  }

  const String cmd = extractStringField(payload, "cmd");
  if (cmd == "ping") {
    sendOkString(id, "pong");
    return;
  }

  if (cmd == "capabilities") {
    sendOkJson(id, "{\"gpio\":[0,1,2,3,4,5,6,7,8,9,10,11,12,13],\"led_pin\":13}");
    return;
  }

  const long pin = extractIntField(payload, "pin", -1);
  if (pin < 0 || pin > 13) {
    sendError(id, "pin must be between 0 and 13");
    return;
  }

  if (cmd == "gpio_read") {
    pinMode((int)pin, INPUT);
    const int value = digitalRead((int)pin) == HIGH ? 1 : 0;
    sendOkString(id, String(value));
    return;
  }

  if (cmd == "gpio_write") {
    const long value = extractIntField(payload, "value", -1);
    if (value != 0 && value != 1) {
      sendError(id, "value must be 0 or 1");
      return;
    }
    pinMode((int)pin, OUTPUT);
    digitalWrite((int)pin, value == 1 ? HIGH : LOW);
    sendOkString(id, "done");
    return;
  }

  sendError(id, "unsupported command");
}

void setup() {
  Serial.begin(115200);
  pinMode(kLedPin, OUTPUT);
  g_input.reserve(256);
}

void loop() {
  while (Serial.available() > 0) {
    const char c = (char)Serial.read();
    if (c == '\r') {
      continue;
    }
    if (c == '\n') {
      if (g_input.length() > 0) {
        handleRequest(g_input);
        g_input = "";
      }
      continue;
    }

    if (g_input.length() < 255) {
      g_input += c;
    }
  }
}
