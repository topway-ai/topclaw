# Runbook heartbeat

Dùng tài liệu này khi bạn muốn heartbeat của daemon TopClaw hoạt động giống kiểu theo dõi định kỳ có trí nhớ, thay vì lặp lại mù quáng theo timer.

## Điều gì thay đổi

Heartbeat task bây giờ có trạng thái:

- dòng `- task` cũ vẫn dùng được
- mỗi task lưu lịch sử chạy trong `state/heartbeat_state.json`
- mỗi task có cooldown thay vì chạy lại mù quáng ở mọi tick
- lỗi lặp lại sẽ tự động backoff
- task kiểu `max_runs=1` sẽ dừng sau khi chạy đủ số lần
- mỗi tick chỉ chạy các task tới hạn có ưu tiên cao nhất, thay vì chạy tất cả cùng lúc

## Cú pháp HEARTBEAT.md

Task cơ bản:

```md
- Review my calendar
```

Task có metadata:

```md
- [every=4h] [priority=2] Review my calendar for the next 24 hours
- [every=1d] Check active repos for stale branches
- [every=30m] [max_runs=1] Remind me to finish onboarding notes
```

Metadata hỗ trợ:

- `every=<duration>` hoặc `cooldown=<duration>`
- `priority=<integer>`
- `max_runs=<integer>`

Ví dụ duration:

- `30m`
- `4h`
- `1d`

## Hành vi vận hành

- Nếu `HEARTBEAT.md` có task, TopClaw sẽ lên lịch từ đó.
- Nếu `HEARTBEAT.md` không có bullet task, TopClaw fallback sang `heartbeat.message` nếu được cấu hình.
- Task mới sẽ tới hạn ngay.
- Task chạy thành công sẽ đặt `next_due_at` theo cooldown.
- Task lỗi sẽ retry sớm lúc đầu, rồi backoff dần.
- Task bị xóa khỏi file sẽ không còn được chọn; state cũ có thể vẫn còn trong file state như lịch sử.

## Tệp liên quan

- Nguồn task: `<workspace>/HEARTBEAT.md`
- State task: `<workspace>/state/heartbeat_state.json`
- Snapshot health daemon: `~/.topclaw/daemon_state.json`

## Khuyến nghị

- Giữ task heartbeat nhỏ, rõ ràng, cụ thể.
- Nên có 3-10 task bền vững, không phải một danh sách mong muốn quá dài.
- Chỉ dùng `priority=2` trở lên cho việc thật sự cần chạy trước.
- Dùng `max_runs=1` cho nhắc việc một lần hoặc reminder migration.
- Không nên nhét shell command phá hoại trực tiếp vào prompt heartbeat.

## Kiểm tra nhanh

1. Khởi động daemon.
2. Thêm một task test vào `HEARTBEAT.md`.
3. Chờ một heartbeat interval.
4. Xác nhận `state/heartbeat_state.json` có `last_run_at`, `next_due_at`, và các bộ đếm.
5. Xác nhận cùng task đó không bị chạy lại ngay ở tick kế tiếp nếu chưa tới hạn.

## Rollback

Nếu cadence mới không đúng ý:

1. Dừng daemon.
2. Đơn giản hóa `HEARTBEAT.md` về các dòng `- task` thuần.
3. Xóa `<workspace>/state/heartbeat_state.json` nếu muốn reset trí nhớ heartbeat.
4. Khởi động lại daemon.
