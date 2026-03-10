# Khắc phục sự cố TopClaw

Các lỗi thường gặp khi cài đặt và chạy, kèm cách khắc phục.

Xác minh lần cuối: **2026-03-09**.

## Kiểm tra nhanh

| Nếu gặp tình huống này | Hãy bắt đầu ở đây |
|---|---|
| Cài xong nhưng không tìm thấy lệnh `topclaw` | [Không tìm thấy lệnh `topclaw` sau cài đặt](#không-tìm-thấy-lệnh-topclaw-sau-cài-đặt) |
| Onboarding xong nhưng TopClaw vẫn không trả lời | [Onboarding xong nhưng TopClaw vẫn không phản hồi](#onboarding-xong-nhưng-topclaw-vẫn-không-phản-hồi) |
| Channel đã cấu hình nhưng runtime nền không chạy | [Dịch vụ đã cài nhưng không chạy](#dịch-vụ-đã-cài-nhưng-không-chạy) |
| Auth provider bị thiếu hoặc hết hạn | [Auth provider bị thiếu hoặc hết hạn](#auth-provider-bị-thiếu-hoặc-hết-hạn) |

## Cài đặt / Bootstrap

### Không tìm thấy `cargo`

Triệu chứng:

- bootstrap thoát với lỗi `cargo is not installed`

Khắc phục:

```bash
./bootstrap.sh --install-rust
```

Hoặc cài từ <https://rustup.rs/>.

### Thiếu thư viện hệ thống để build

Triệu chứng:

- build thất bại do lỗi trình biên dịch hoặc `pkg-config`

Khắc phục:

```bash
./bootstrap.sh --install-system-deps
```

### Build thất bại trên máy ít RAM / ít dung lượng

Triệu chứng:

- `cargo build --release` bị kill (`signal: 9`, OOM killer, hoặc `cannot allocate memory`)
- Build vẫn lỗi sau khi thêm swap vì hết dung lượng ổ đĩa

Nguyên nhân:

- RAM lúc chạy (<5MB) khác xa RAM lúc biên dịch.
- Build đầy đủ từ mã nguồn có thể cần **2 GB RAM + swap** và **6+ GB dung lượng trống**.
- Bật swap trên ổ nhỏ có thể tránh OOM RAM nhưng vẫn lỗi vì hết dung lượng.

Cách tốt nhất cho máy hạn chế tài nguyên:

```bash
./bootstrap.sh --prefer-prebuilt
```

Chế độ chỉ dùng binary (không build từ nguồn):

```bash
./bootstrap.sh --prebuilt-only
```

Nếu bắt buộc phải build từ nguồn trên máy yếu:

1. Chỉ thêm swap nếu còn đủ dung lượng cho cả swap lẫn kết quả build.
1. Giới hạn số luồng build:

```bash
CARGO_BUILD_JOBS=1 cargo build --release --locked
```

1. Bỏ bớt feature nặng khi không cần Matrix:

```bash
cargo build --release --locked --no-default-features --features hardware
```

1. Cross-compile trên máy mạnh hơn rồi copy binary sang máy đích.

### Build rất chậm hoặc có vẻ bị treo

Triệu chứng:

- `cargo check` / `cargo build` dừng lâu ở `Checking topclaw`
- Lặp lại thông báo `Blocking waiting for file lock on package cache` hoặc `build directory`

Nguyên nhân:

- Thư viện Matrix E2EE (`matrix-sdk`, `ruma`, `vodozemac`) lớn và tốn thời gian kiểm tra kiểu.
- TLS + crypto native build script (`aws-lc-sys`, `ring`) tăng thời gian biên dịch đáng kể.
- `rusqlite` với SQLite tích hợp biên dịch mã C cục bộ.
- Chạy nhiều cargo job/worktree song song gây tranh chấp file lock.

Kiểm tra nhanh:

```bash
cargo check --timings
cargo tree -d
```

Báo cáo thời gian được ghi tại `target/cargo-timings/cargo-timing.html`.

Lặp nhanh hơn khi không cần kênh Matrix:

```bash
cargo check --no-default-features --features hardware
```

Lệnh này bỏ qua `channel-matrix` và giảm đáng kể thời gian biên dịch.

Build với Matrix:

```bash
cargo check --no-default-features --features hardware,channel-matrix
```

Giảm tranh chấp lock:

```bash
pgrep -af "cargo (check|build|test)|cargo check|cargo build|cargo test"
```

Dừng các cargo job không liên quan trước khi build.

### Không tìm thấy lệnh `topclaw` sau cài đặt

Triệu chứng:

- Cài đặt thành công nhưng shell không tìm thấy `topclaw`

Khắc phục:

```bash
export PATH="$HOME/.cargo/bin:$PATH"
which topclaw
```

Thêm vào shell profile nếu cần giữ lâu dài.

### Onboarding xong nhưng TopClaw vẫn không phản hồi

Kiểm tra theo đúng thứ tự sau:

```bash
topclaw status
topclaw status --diagnose
topclaw service status
topclaw channel doctor
```

Hãy chú ý:

- provider chưa auth xong
- service chưa cài hoặc chưa chạy
- channel token / allowlist còn thiếu
- nền tảng của bạn cần thiết lập service thủ công

Nếu `topclaw service status` cho biết chưa có runtime nền:

```bash
topclaw service install
topclaw service start
```

## Runtime / Gateway

### Không kết nối được gateway

Kiểm tra:

```bash
topclaw status
topclaw doctor
```

Xác minh `~/.topclaw/config.toml`:

- `[gateway].host` (mặc định `127.0.0.1`)
- `[gateway].port` (mặc định `3000`)
- `allow_public_bind` chỉ bật khi cố ý mở truy cập LAN/public

### Auth provider bị thiếu hoặc hết hạn

Kiểm tra:

```bash
topclaw status
topclaw status --diagnose
```

Thường có nghĩa là:

- provider vẫn cần đăng nhập OAuth/subscription
- API key chưa được thiết lập
- auth đã lưu bị hết hạn và cần làm mới

Khắc phục:

1. Đọc chính xác lệnh auth tiếp theo mà `topclaw status --diagnose` gợi ý.
2. Hoàn tất đăng nhập provider hoặc đặt đúng API key.
3. Chạy lại `topclaw status` để xác nhận provider đã sẵn sàng.

### Lỗi ghép nối / xác thực webhook

Kiểm tra:

1. Đảm bảo đã hoàn tất ghép nối (luồng `/pair`)
2. Đảm bảo bearer token còn hiệu lực
3. Chạy lại chẩn đoán:

```bash
topclaw doctor
```

## Sự cố kênh

### Telegram xung đột: `terminated by other getUpdates request`

Nguyên nhân:

- Nhiều poller dùng chung bot token

Khắc phục:

- Chỉ giữ một runtime đang chạy cho token đó
- Dừng các tiến trình `topclaw daemon` / `topclaw channel start` thừa

### Kênh không khỏe trong `channel doctor`

Kiểm tra:

```bash
topclaw channel doctor
```

Sau đó xác minh thông tin xác thực và trường allowlist cho từng kênh trong config.

Nếu channel đã cấu hình đúng nhưng vẫn không phản hồi, hãy xác nhận có runtime đang chạy. Với vận hành bình thường, ưu tiên `topclaw service status` hơn `topclaw channel start`.

## Chế độ dịch vụ

Để TopClaw luôn chạy nền:

```bash
topclaw service install
topclaw service start
topclaw service status
```

Để khởi động lại sạch:

```bash
topclaw service stop
topclaw service start
```

### Dịch vụ đã cài nhưng không chạy

Kiểm tra:

```bash
topclaw service status
```

Khôi phục:

```bash
topclaw service stop
topclaw service start
```

Xem log trên Linux:

```bash
journalctl --user -u topclaw.service -f
```

## Tương thích cài đặt cũ

Cả hai cách vẫn hoạt động:

```bash
curl -fsSL https://raw.githubusercontent.com/topway-ai/TopClaw/main/scripts/bootstrap.sh | bash
curl -fsSL https://raw.githubusercontent.com/topway-ai/TopClaw/main/scripts/install.sh | bash
```

`install.sh` là điểm vào tương thích, chuyển tiếp/dự phòng về hành vi bootstrap.

## Vẫn chưa giải quyết được?

Thu thập và đính kèm các thông tin sau khi tạo issue:

```bash
topclaw --version
topclaw status
topclaw doctor
topclaw channel doctor
```

Kèm thêm: hệ điều hành, cách cài đặt, và đoạn config đã ẩn bí mật.

## Tài liệu liên quan

- [operations-runbook.md](operations-runbook.md)
- [one-click-bootstrap.md](one-click-bootstrap.md)
- [channels-reference.md](channels-reference.md)
- [network-deployment.md](network-deployment.md)
