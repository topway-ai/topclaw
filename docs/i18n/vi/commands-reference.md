# Tham khảo lệnh TopClaw (Tiếng Việt)

Đây là trang cầu nối để tra cứu nhanh các lệnh CLI của TopClaw.

Nguồn tiếng Anh:

- [../../commands-reference.md](../../commands-reference.md)

## Dùng khi nào

- Tìm lệnh theo tác vụ (`bootstrap`, `status`, `doctor`, `channel`, `service`)
- Kiểm tra alias và bề mặt CLI hiện tại
- Xác nhận hành vi lệnh khi tài liệu tiếng Anh vừa được cập nhật

## Quy ước

- Tên lệnh, cờ CLI, tên config và hành vi runtime chuẩn đều lấy theo tài liệu tiếng Anh.
- Trang này chỉ giữ vai trò định hướng; hợp đồng CLI đầy đủ nằm ở bản tiếng Anh.

## Điểm cần chú ý

- `topclaw bootstrap` là lệnh thiết lập chuẩn cho phiên bản hiện tại.
- Với channel chạy nền, ưu tiên `topclaw service status` để kiểm tra trạng thái trước khi cài lại service.
- Sau khi onboarding xong và đã cấu hình channel, bước thử đầu tiên nên là nhắn tin cho bot trong channel đó.
