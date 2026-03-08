# Tham khảo lệnh TopClaw

Dựa trên CLI hiện tại (`topclaw --help`).

Xác minh lần cuối: **2026-03-07**.

## Lệnh cấp cao nhất

| Lệnh | Mục đích |
|---|---|
| `onboard` | Khởi tạo workspace/config nhanh hoặc tương tác |
| `agent` | Chạy chat tương tác hoặc chế độ gửi tin nhắn đơn |
| `gateway` | Khởi động gateway webhook và HTTP WhatsApp |
| `daemon` | Khởi động runtime có giám sát (gateway + channels + heartbeat/scheduler tùy chọn) |
| `service` | Quản lý vòng đời dịch vụ cấp hệ điều hành |
| `doctor` | Chạy chẩn đoán và kiểm tra trạng thái |
| `status` | Hiển thị cấu hình và tóm tắt hệ thống |
| `update` | Kiểm tra hoặc cài bản phát hành TopClaw mới nhất |
| `cron` | Quản lý tác vụ định kỳ |
| `models` | Làm mới danh mục model của provider |
| `providers` | Liệt kê ID provider, bí danh và provider đang dùng |
| `channel` | Quản lý kênh và kiểm tra sức khỏe kênh |
| `integrations` | Kiểm tra chi tiết tích hợp |
| `skills` | Liệt kê/cài đặt/gỡ bỏ skills |
| `migrate` | Nhập dữ liệu từ runtime khác (hiện hỗ trợ OpenClaw) |
| `config` | Xuất schema cấu hình dạng máy đọc được |
| `completions` | Tạo script tự hoàn thành cho shell ra stdout |
| `hardware` | Phát hiện và kiểm tra phần cứng USB |
| `peripheral` | Cấu hình và nạp firmware thiết bị ngoại vi |

Bí danh thông dụng:

- `topclaw init` -> `topclaw onboard`
- `topclaw chat` -> `topclaw agent`
- `topclaw run` -> `topclaw daemon`
- `topclaw info` -> `topclaw status`
- `topclaw channels` -> `topclaw channel`
- `topclaw skill` -> `topclaw skills`

## Nhóm lệnh

### `onboard`

- `topclaw onboard`
- `topclaw onboard --interactive`
- `topclaw onboard --channels-only`
- `topclaw onboard --api-key <KEY> --provider <ID> --memory <sqlite|lucid|markdown|none>`
- `topclaw onboard --api-key <KEY> --provider <ID> --model <MODEL_ID> --memory <sqlite|lucid|markdown|none>`

### `agent`

- `topclaw agent`
- `topclaw agent -m "Hello"`
- `topclaw agent --provider <ID> --model <MODEL> --temperature <0.0-2.0>`
- `topclaw agent --peripheral <board:path>`

### `gateway` / `daemon`

- `topclaw gateway [--host <HOST>] [--port <PORT>] [--new-pairing]`
- `topclaw daemon [--host <HOST>] [--port <PORT>]`

`--new-pairing` sẽ xóa toàn bộ token đã ghép đôi và tạo mã ghép đôi mới khi gateway khởi động.

### `service`

- `topclaw service install`
- `topclaw service start`
- `topclaw service stop`
- `topclaw service restart`
- `topclaw service status`
- `topclaw service uninstall`

### `update`

- `topclaw update`
- `topclaw update --check`
- `topclaw update --force`

Ghi chú:

- `topclaw update` tải bản phát hành GitHub chính thức mới nhất phù hợp với nền tảng hiện tại và thay thế binary đang dùng.
- `--check` chỉ kiểm tra xem có bản mới hay không.
- `--force` cài lại bản mới nhất kể cả khi phiên bản hiện tại đã trùng.
- Sau khi cập nhật trên máy đang chạy TopClaw như dịch vụ nền, hãy chạy `topclaw service restart`.
- Nếu vị trí binary không cho phép ghi, TopClaw sẽ in ra hướng dẫn khôi phục thay vì chỉ báo lỗi chung chung. Trên Linux, phương án khuyến nghị là dùng trình cài đặt phát hành chính thức:

```bash
curl -fsSL https://raw.githubusercontent.com/jackfly8/TopClaw/main/scripts/install-release.sh | bash
```

### `cron`

- `topclaw cron list`
- `topclaw cron add <expr> [--tz <IANA_TZ>] <command>`
- `topclaw cron add-at <rfc3339_timestamp> <command>`
- `topclaw cron add-every <every_ms> <command>`
- `topclaw cron once <delay> <command>`
- `topclaw cron remove <id>`
- `topclaw cron pause <id>`
- `topclaw cron resume <id>`

### `models`

- `topclaw models refresh`
- `topclaw models refresh --provider <ID>`
- `topclaw models refresh --force`

`models refresh` hiện hỗ trợ làm mới danh mục trực tiếp cho các provider: `openrouter`, `openai`, `anthropic`, `groq`, `mistral`, `deepseek`, `xai`, `together-ai`, `gemini`, `ollama`, `astrai`, `venice`, `fireworks`, `cohere`, `moonshot`, `glm`, `zai`, `qwen` và `nvidia`.

### `doctor`

- `topclaw doctor`

`topclaw doctor` hiện kết thúc bằng các lệnh bước tiếp theo cụ thể khi phát hiện các vấn đề thiết lập có thể xử lý ngay, như thiếu provider, thiếu xác thực, chưa cấu hình channel, hoặc thiếu thư mục workspace.

### `status`

- `topclaw status`

`topclaw status` hiển thị tóm tắt cấu hình/runtime hiện tại và giờ cũng gợi ý các lệnh bước tiếp theo cho những thiếu sót quan trọng trong cấu hình, dùng cùng logic gợi ý với `topclaw doctor`.

### `channel`

- `topclaw channel list`
- `topclaw channel start`
- `topclaw channel doctor`
- `topclaw channel bind-telegram <IDENTITY>`
- `topclaw channel add <type> <json>`
- `topclaw channel remove <name>`

Lệnh trong chat khi runtime đang chạy (Telegram/Discord):

- `/models`
- `/models <provider>`
- `/model`
- `/model <model-id>`

Channel runtime cũng theo dõi `config.toml` và tự động áp dụng thay đổi cho:
- `default_provider`
- `default_model`
- `default_temperature`
- `api_key` / `api_url` (cho provider mặc định)
- `reliability.*` cài đặt retry của provider

`add/remove` hiện chuyển hướng về thiết lập có hướng dẫn / cấu hình thủ công (chưa hỗ trợ đầy đủ mutator khai báo).

### `integrations`

- `topclaw integrations info <name>`

### `skills`

- `topclaw skills list`
- `topclaw skills install <source>`
- `topclaw skills remove <name>`

`<source>` chấp nhận git remote (`https://...`, `http://...`, `ssh://...` và `git@host:owner/repo.git`) hoặc đường dẫn cục bộ.

Skill manifest (`SKILL.toml`) hỗ trợ `prompts` và `[[tools]]`; cả hai được đưa vào system prompt của agent khi chạy, giúp model có thể tuân theo hướng dẫn skill mà không cần đọc thủ công.

### `migrate`

- `topclaw migrate openclaw [--source <path>] [--dry-run]`

### `config`

- `topclaw config schema`

`config schema` xuất JSON Schema (draft 2020-12) cho toàn bộ hợp đồng `config.toml` ra stdout.

### `completions`

- `topclaw completions bash`
- `topclaw completions fish`
- `topclaw completions zsh`
- `topclaw completions powershell`
- `topclaw completions elvish`

`completions` chỉ xuất ra stdout để script có thể được source trực tiếp mà không bị lẫn log/cảnh báo.

### `hardware`

- `topclaw hardware discover`
- `topclaw hardware introspect <path>`
- `topclaw hardware info [--chip <chip_name>]`

### `peripheral`

- `topclaw peripheral list`
- `topclaw peripheral add <board> <path>`
- `topclaw peripheral flash [--port <serial_port>]`
- `topclaw peripheral setup-uno-q [--host <ip_or_host>]`
- `topclaw peripheral flash-nucleo`

## Kiểm tra nhanh

Để xác minh nhanh tài liệu với binary hiện tại:

```bash
topclaw --help
topclaw <command> --help
```
