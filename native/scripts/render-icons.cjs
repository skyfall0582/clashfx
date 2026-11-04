// Optional asset tool. Requires sharp; the application build uses the checked-in exports.
const fs = require('node:fs/promises');
const path = require('node:path');
const sharp = require(process.env.CLASHFX_SHARP_MODULE || 'sharp');

async function writeIcon(source, destination) {
    const sizes = [16, 20, 24, 32, 40, 48, 64, 128, 256];
    const frames = await Promise.all(sizes.map(size => sharp(source, { density: 384 })
        .resize(size, size).png().toBuffer()));
    const header = Buffer.alloc(6 + sizes.length * 16);
    header.writeUInt16LE(1, 2);
    header.writeUInt16LE(sizes.length, 4);
    let offset = header.length;
    frames.forEach((frame, index) => {
        const entry = 6 + index * 16;
        header[entry] = sizes[index] === 256 ? 0 : sizes[index];
        header[entry + 1] = header[entry];
        header.writeUInt16LE(1, entry + 4);
        header.writeUInt16LE(32, entry + 6);
        header.writeUInt32LE(frame.length, entry + 8);
        header.writeUInt32LE(offset, entry + 12);
        offset += frame.length;
    });
    await fs.writeFile(destination, Buffer.concat([header, ...frames]));
}

async function main() {
    const assets = path.resolve(__dirname, '../assets');
    const source = await fs.readFile(path.join(assets, 'clashfx.svg'));
    await writeIcon(source, path.join(assets, 'clashfx.ico'));
    for (const [state, color] of [
        ['off', '#E94F47'],
        ['on', '#1FAA70'],
        ['busy', '#3B82F6'],
    ]) {
        const badge = `<circle cx="198" cy="58" r="46" fill="${color}" stroke="white" stroke-width="6"/>`;
        const tray = Buffer.from(source.toString().replace('</svg>', `${badge}</svg>`));
        await writeIcon(tray, path.join(assets, `clashfx-tray-${state}.ico`));
    }
    await sharp(source, { density: 384 }).resize(512, 512).png()
        .toFile(path.join(assets, 'clashfx.png'));
    await sharp(source, { density: 384 }).resize(128, 128).png()
        .toFile(path.join(assets, 'clashfx-sidebar.png'));
    await sharp(path.join(assets, 'connection-orbit.svg'), { density: 192 }).png()
        .toFile(path.join(assets, 'connection-orbit.png'));
    console.log('Exported application icons and connection artwork.');
}

main().catch(error => { console.error(error); process.exitCode = 1; });
