# learn

[learn.xabcnews.com](https://learn.xabcnews.com)（XABC 学习）是一个只做一件事的网站：系统地学 AI 与硬件开发。课程由 XABC 一课接一课地写，并按官方最新文档持续修订。它是 [xabcnews.com](https://xabcnews.com) 的姊妹站，2026-10-27 从 news 的「学习」板块独立出来，原有课程连同课文、配图和课号一起迁了过来。

## 网站

- `/`：学习地图。课程按类别分组，列出课数和最近更新。
- `/{课程}`：课程页。左边是全部课程，中间是简介、最近更新和按章排列的目录，右边是章节索引。
- `/{课程}/{课号}`：课文页，例如 `/esp32/12`。左边是这门课的目录，当前章展开、当前课高亮；中间是课文；右边是本课的二、三级标题，读到哪里就标到哪里。课文末尾有上一课和下一课。手机上只保留中间一栏，目录收进正文上方的「课程目录」。
- `/sitemap.xml`、`/robots.txt`。

页面只用中文，全部由服务端生成，每位读者看到的都一样。页面带 ETag，缓存头为 `public, max-age=60, stale-while-revalidate=300`。浏览器脚本只做三件事：代码块的「复制」按钮、右栏标出当前小节、回到顶部。CSP 只允许本站的脚本和样式，图片来自 tmpfile。

## XABC 怎么写课

每 15 分钟一步，先维护再扩展：

1. **修订**：有课文到了复查日期，就修订到期最久的那一课。复查周期由计划给出：讲命令、界面、版本和价格的 30–60 天，讲原理和方法的 90–180 天。修订保留课号、章、封面和原有配图，可以挪动或删掉旧图，也可以加新图，修订号加一。
2. **新课**：没有到期的课，就给「最新一课最旧」的课程写下一课，还没有课的课程优先。先做计划（章、标题、目标、要点、复查天数），再用网页检索官方文档、仓库、版本说明和高质量教程，然后整课写成。
3. **配图**：课文用 `![](img:N)` 单独成段地放图，图由图像模型按描述绘制，转成 JPEG 存到 tmpfile，图下自动加图注「（AI 生成）」。封面同样是绘制的宽幅插图。
4. **链接**：课文只能链接到检索中实际看到的页面和本站课程，其余链接一律只保留文字；检索答复在句末留下的「([example.com](…))」来源标记会被去掉。
5. **开课**：每周一次，用网页检索看最近两周持续升温、读者能学会能动手、而且现有课程覆盖不了的方向。依据必须包含至少两个检索中实际看到的页面，否则不开。可见课程最多 24 门。

失败后等半小时再试，连续失败每次加倍，最长 8 小时。

## 命令

没有后台。需要人来决定的事，在服务器上用命令完成：

```sh
docker exec learn ./learn open "一句话说想开的课"
docker exec learn ./learn turn esp32 "一句话说怎么调整"
docker exec learn ./learn hide esp32
docker exec learn ./learn show esp32
docker exec learn ./learn write      # 马上写一课
docker exec learn ./learn courses    # 列出全部课程
```

`turn` 会改写课程说明；如果调整让已写的课出错或过时，这门课的课文会全部变为到期，由 XABC 逐课修订。

## 数据

`migrations/` 定义三张表：

- `courses`：课程的地址、类别、名称、简介、给 XABC 的课程说明，以及来源（`seed`、`request`、`discovery`）、是否显示和最近一次调整。
- `lessons`：课程内的课号、章、标题、Markdown 正文、摘要、封面、修订号、复查日期和发布与更新时间。
- `images`：课文的配图，存在 tmpfile 上，正文用 `image:<id>` 引用。

另有 `tasks` 记录每周任务上次运行的日期。

## 开发与验证

```sh
cp .env.example .env
make release-check
LEARN_TEST_DATABASE_URL=postgres://.../learn_test_check make integration-test
```

`release-check` 执行格式检查、单元测试、Clippy、前端语法和 Compose 配置检查。集成测试需要一个名称以 `learn_test` 开头的空库，用本地模拟的 AI 网关和 tmpfile 跑完整流程：一句话开课、写课与绘图、页面、到期修订、调整方向和隐藏课程。CI 自动执行。

## 部署

服务器上的 `~/1/learn` 只放一份 `docker-compose.yml`：与仓库里的相同，`AI_API_KEY` 与 `TMPFILE_API_KEY` 直接写成实际的值。推送到 `main` 会构建 `proxyxai/learn:rust` 镜像；提交信息含 `prod` 时再部署到生产。网站监听 `127.0.0.1:8794`，由网关 nginx 以 `learn.xabcnews.com` 对外提供，Cloudflare 在前。

## 许可证

本项目按 [AGPL-3.0](LICENSE) 授权。以修改后的版本对外提供网络服务时，须按同一许可证向用户公开完整源码。XABC 这一名称与站点图标不在授权范围内。
