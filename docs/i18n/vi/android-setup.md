# Thiết lập Android (Tiếng Việt)

Trang này là bản địa hóa tối thiểu cho hướng dẫn Android.

Bản gốc tiếng Anh:

- [../../android-setup.md](../../android-setup.md)

## Tóm tắt nhanh

- Hỗ trợ kiến trúc `armv7-linux-androideabi` và `aarch64-linux-android`.
- Cách dễ nhất: chạy qua Termux.
- Có thể dùng ADB cho luồng nâng cao.

## Điểm cần kiểm tra

1. Xác định đúng kiến trúc thiết bị (`uname -m`).
2. Tải đúng binary theo kiến trúc.
3. Kiểm tra quyền thực thi (`chmod +x topclaw`).
4. Chạy `topclaw --version` và `topclaw onboard` để xác minh.

Nếu cần lệnh chi tiết đầy đủ, dùng bản gốc tiếng Anh ở liên kết trên.
