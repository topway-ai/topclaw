// TopClaw Arduino UNO Q bridge sketch.
//
// Protocol over serial:
//   ping
//   gpio_read <pin>
//   gpio_write <pin> <0|1>

String g_input;

void replyLine(const String& line) {
  Serial.println(line);
}

long parseLongSafe(const String& value, long fallback) {
  if (value.length() == 0) {
    return fallback;
  }
  return value.toInt();
}

void handleCommand(String line) {
  line.trim();
  if (line.length() == 0) {
    return;
  }

  const int first_space = line.indexOf(' ');
  const String cmd = first_space >= 0 ? line.substring(0, first_space) : line;
  const String rest = first_space >= 0 ? line.substring(first_space + 1) : "";

  if (cmd == "ping") {
    replyLine("pong");
    return;
  }

  if (cmd == "gpio_read") {
    const long pin = parseLongSafe(rest, -1);
    if (pin < 0) {
      replyLine("error: missing pin");
      return;
    }
    pinMode((int)pin, INPUT);
    replyLine(digitalRead((int)pin) == HIGH ? "1" : "0");
    return;
  }

  if (cmd == "gpio_write") {
    const int second_space = rest.indexOf(' ');
    if (second_space < 0) {
      replyLine("error: missing value");
      return;
    }

    const long pin = parseLongSafe(rest.substring(0, second_space), -1);
    const long value = parseLongSafe(rest.substring(second_space + 1), -1);
    if (pin < 0 || (value != 0 && value != 1)) {
      replyLine("error: invalid gpio_write args");
      return;
    }

    pinMode((int)pin, OUTPUT);
    digitalWrite((int)pin, value == 1 ? HIGH : LOW);
    replyLine("done");
    return;
  }

  replyLine("error: unsupported command");
}

void setup() {
  Serial.begin(115200);
  g_input.reserve(128);
}

void loop() {
  while (Serial.available() > 0) {
    const char c = (char)Serial.read();
    if (c == '\r') {
      continue;
    }
    if (c == '\n') {
      if (g_input.length() > 0) {
        handleCommand(g_input);
        g_input = "";
      }
      continue;
    }

    if (g_input.length() < 127) {
      g_input += c;
    }
  }
}
