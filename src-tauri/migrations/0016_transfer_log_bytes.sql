-- 断点续传（用户 2026-09-07 需求："SFTP 传输需要支持可靠传输和断点续传功能"）：
-- 记录这次传输结束时已经确认写入的字节数和文件总字节数，供传输历史展示"传到哪儿
-- 断的"。真正决定续传起点的是本地/远程文件当时的实际大小（见
-- `fsops::remote::download_range_to_local`/`upload_range_from_local`），这两列只是
-- 给人看的诊断信息，不参与续传逻辑本身，旧记录里是 NULL 也没关系。
ALTER TABLE transfer_log ADD COLUMN bytes_transferred INTEGER;
ALTER TABLE transfer_log ADD COLUMN total_bytes INTEGER;
