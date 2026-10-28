import AVFoundation
import CoreMedia

/// Converts ScreenCaptureKit audio CMSampleBuffers into AVAudioPCMBuffers so they can be
/// appended to an AVAudioFile alongside the AVAudioEngine-captured microphone track.
enum AudioSampleConverter {
    static func pcmBuffer(from sampleBuffer: CMSampleBuffer) -> AVAudioPCMBuffer? {
        guard let formatDescription = CMSampleBufferGetFormatDescription(sampleBuffer),
              let asbdPointer = CMAudioFormatDescriptionGetStreamBasicDescription(formatDescription) else {
            return nil
        }
        guard let format = AVAudioFormat(streamDescription: asbdPointer) else { return nil }

        let frameCount = AVAudioFrameCount(CMSampleBufferGetNumSamples(sampleBuffer))
        guard frameCount > 0,
              let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: frameCount) else {
            return nil
        }
        buffer.frameLength = frameCount

        // AudioBufferList is a variable-length C struct with one static `mBuffers` slot
        // in the Swift import — a bare `AudioBufferList()` only has room for 1 buffer.
        // ScreenCaptureKit's system audio is commonly non-interleaved multi-channel,
        // which needs one buffer per channel; passing a too-small list here made this
        // call fail silently for every sample (root cause of system audio always
        // capturing 0 bytes — see
        // docs/superpowers/specs/2026-07-25-reliable-daily-driver-design.md).
        let maxBuffers = max(Int(asbdPointer.pointee.mChannelsPerFrame), 1)
        let bufferList = AudioBufferList.allocate(maximumBuffers: maxBuffers)
        defer { free(bufferList.unsafeMutablePointer) }
        let bufferListByteSize = AudioBufferList.sizeInBytes(maximumBuffers: maxBuffers)

        var blockBuffer: CMBlockBuffer?
        let status = CMSampleBufferGetAudioBufferListWithRetainedBlockBuffer(
            sampleBuffer,
            bufferListSizeNeededOut: nil,
            bufferListOut: bufferList.unsafeMutablePointer,
            bufferListSize: bufferListByteSize,
            blockBufferAllocator: nil,
            blockBufferMemoryAllocator: nil,
            flags: kCMSampleBufferFlag_AudioBufferList_Assure16ByteAlignment,
            blockBufferOut: &blockBuffer
        )
        guard status == noErr else { return nil }

        // Every `mData` pointer filled in above points into `blockBuffer`'s storage, and
        // `blockBuffer` is otherwise never read again — ARC is free to release it before
        // the memcpy loop runs, leaving those pointers dangling. Harmless under -Onone
        // but a real use-after-free under -O (the notarized Release build), so the copy
        // and the return are kept inside withExtendedLifetime.
        return withExtendedLifetime(blockBuffer) {
            let destinationBufferList = UnsafeMutableAudioBufferListPointer(buffer.mutableAudioBufferList)
            for (source, destination) in zip(bufferList, destinationBufferList) {
                guard let sourceData = source.mData, let destinationData = destination.mData else { continue }
                memcpy(destinationData, sourceData, min(Int(source.mDataByteSize), Int(destination.mDataByteSize)))
            }
            return buffer
        }
    }
}
