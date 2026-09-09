//! Output transport shared by one-shot and interactive presentation drivers.

pub(super) enum OutputWork {
    Write(Vec<u8>),
    Flush,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum OutputAcknowledgement {
    Succeeded,
    Failed,
}

pub(super) struct OutputBridge {
    pub(super) work: tokio::sync::mpsc::Sender<OutputWork>,
    pub(super) acknowledgements: tokio::sync::mpsc::Receiver<OutputAcknowledgement>,
}

pub(super) fn serve_output(
    mut work: tokio::sync::mpsc::Receiver<OutputWork>,
    acknowledgements: &tokio::sync::mpsc::Sender<OutputAcknowledgement>,
    output: &mut dyn std::io::Write,
) {
    while let Some(work) = work.blocking_recv() {
        let succeeded = match work {
            OutputWork::Write(bytes) => output.write_all(&bytes),
            OutputWork::Flush => output.flush(),
        }
        .is_ok();
        let acknowledgement = if succeeded {
            OutputAcknowledgement::Succeeded
        } else {
            OutputAcknowledgement::Failed
        };
        if acknowledgements.blocking_send(acknowledgement).is_err() {
            break;
        }
    }
}
