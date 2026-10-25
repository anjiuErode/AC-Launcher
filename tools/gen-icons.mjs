// 生成 ACL 启动器图标集
// 应用图标：Minecraft 蜜蜂全身（透明底，保留 Alpha）
// 界面品牌图：瀛洲公会 Logo（保持原样，不由本脚本重绘）
// 用法: node tools/gen-icons.mjs
import sharp from 'sharp'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

// 路径从脚本位置推导，项目目录改名后无需改动本文件
const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')

const BRAND_SRC = 'D:/Admin/Pictures/瀛洲公会LOGO文字-透明 (1).png'
const BEE_SRC = path.join(ROOT, 'tools/assets/bee_full.png')
// 素材不入库（Mojang 资产），缺失时自动下载
async function downloadBeeIfMissing() {
	const url = 'https://minecraft.wiki/images/Bee.png'
	if (fs.existsSync(BEE_SRC)) return
	fs.mkdirSync(path.dirname(BEE_SRC), { recursive: true })
	const res = await fetch(url)
	if (!res.ok) throw new Error(`下载素材失败: ${res.status} ${url}`)
	fs.writeFileSync(BEE_SRC, Buffer.from(await res.arrayBuffer()))
	console.log('已下载图标素材:', BEE_SRC)
}

// 图标底色：完全透明（保留 Alpha 通道）
const BEE_BG = { r: 0, g: 0, b: 0, alpha: 0 }
const ICON_DIR = path.join(ROOT, 'apps/app/icons')
const UI_ASSET_DIR = path.join(ROOT, 'apps/app-frontend/src/assets/branding')

const PLATE = 1024

/** 生成圆角矩形白底遮罩 SVG */
function plateSvg(size, radius) {
	return Buffer.from(
		`<svg width="${size}" height="${size}"><rect x="0" y="0" width="${size}" height="${size}" rx="${radius}" ry="${radius}" fill="#ffffff"/></svg>`,
	)
}

/** 应用图标：蜜蜂全身照铺底，白底不透明（alpha 全 255） */
async function buildPlate(size) {
	const box = Math.round(size * 0.94)
	// 小尺寸用最近邻，保留硬像素边缘（避免抗锯齿糊化）；大尺寸用 lanczos 保细节
	const kernel = size <= 64 ? 'nearest' : 'lanczos3'
	let bee = sharp(BEE_SRC).trim({ threshold: 1 }).resize(box, box, { fit: 'inside', kernel })
	if (size > 64 && size <= 128) bee = bee.sharpen({ sigma: 0.6, m1: 0.5, m2: 1.2 })
	const beeBuf = await bee.png().toBuffer()
	const meta = await sharp(beeBuf).metadata()

	return sharp({
		create: { width: size, height: size, channels: 4, background: BEE_BG },
	})
		.composite([
			{
				input: beeBuf,
				top: Math.round((size - meta.height) / 2),
				left: Math.round((size - meta.width) / 2),
			},
		])
		.png({ compressionLevel: 9 })
		.toBuffer()
}

/** 透明底 logo（保留线稿，供界面内使用） */
async function buildTransparent(size, invert) {
	let img = sharp(BRAND_SRC).trim({ threshold: 10 }).resize(size, size, {
		fit: 'inside',
		background: { r: 0, g: 0, b: 0, alpha: 0 },
	})
	if (invert) img = img.negate({ alpha: false })
	return img.png({ compressionLevel: 9 }).toBuffer()
}

/** 打包 ICO（PNG 内嵌，Vista+ 格式） */
async function buildIco(sizes) {
	const images = []
	for (const s of sizes) images.push({ size: s, data: await buildPlate(s) })
	const header = Buffer.alloc(6)
	header.writeUInt16LE(0, 0)
	header.writeUInt16LE(1, 2)
	header.writeUInt16LE(images.length, 4)
	let offset = 6 + images.length * 16
	const entries = []
	for (const img of images) {
		const e = Buffer.alloc(16)
		e.writeUInt8(img.size >= 256 ? 0 : img.size, 0)
		e.writeUInt8(img.size >= 256 ? 0 : img.size, 1)
		e.writeUInt8(0, 2)
		e.writeUInt8(0, 3)
		e.writeUInt16LE(1, 4)
		e.writeUInt16LE(32, 6)
		e.writeUInt32LE(img.data.length, 8)
		e.writeUInt32LE(offset, 12)
		entries.push(e)
		offset += img.data.length
	}
	return Buffer.concat([header, ...entries, ...images.map((i) => i.data)])
}

/** 打包 ICNS */
async function buildIcns() {
	const chunks = [
		['ic07', 128],
		['ic08', 256],
		['ic09', 512],
		['ic10', 1024],
	]
	const bufs = []
	for (const [type, size] of chunks) {
		const png = await buildPlate(size)
		const head = Buffer.alloc(8)
		head.write(type, 0, 4, 'ascii')
		head.writeUInt32BE(png.length + 8, 4)
		bufs.push(head, png)
	}
	const body = Buffer.concat(bufs)
	const head = Buffer.alloc(8)
	head.write('icns', 0, 4, 'ascii')
	head.writeUInt32BE(body.length + 8, 4)
	return Buffer.concat([head, body])
}

async function main() {
	fs.mkdirSync(UI_ASSET_DIR, { recursive: true })
	await downloadBeeIfMissing()

	// 1. 应用图标位图
	const pngMap = {
		'16x16.png': 16,
		'24x24.png': 24,
		'32x32.png': 32,
		'48x48.png': 48,
		'64x64.png': 64,
		'128x128.png': 128,
		'128x128@2x.png': 256,
		'icon.png': 1024,
		'Square30x30Logo.png': 30,
		'Square44x44Logo.png': 44,
		'Square71x71Logo.png': 71,
		'Square89x89Logo.png': 89,
		'Square107x107Logo.png': 107,
		'Square142x142Logo.png': 142,
		'Square150x150Logo.png': 150,
		'Square284x284Logo.png': 284,
		'Square310x310Logo.png': 310,
		'StoreLogo.png': 50,
	}
	for (const [name, size] of Object.entries(pngMap)) {
		fs.writeFileSync(path.join(ICON_DIR, name), await buildPlate(size))
	}

	// 2. ico / icns
	fs.writeFileSync(path.join(ICON_DIR, 'icon.ico'), await buildIco([16, 20, 24, 32, 40, 48, 64, 128, 256]))
	fs.writeFileSync(path.join(ICON_DIR, 'favicon.ico'), await buildIco([16, 32, 48]))
	fs.writeFileSync(path.join(ICON_DIR, 'icon.icns'), await buildIcns())

	// 3. 界面内 logo（黑色 / 白色，透明底）
	fs.writeFileSync(path.join(UI_ASSET_DIR, 'acl-logo-black.png'), await buildTransparent(512, false))
	fs.writeFileSync(path.join(UI_ASSET_DIR, 'acl-logo-white.png'), await buildTransparent(512, true))

	console.log('icons generated ->', ICON_DIR)
	console.log('ui logos generated ->', UI_ASSET_DIR)
}

main().catch((e) => {
	console.error(e)
	process.exit(1)
})
