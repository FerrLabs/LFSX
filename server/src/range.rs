#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Range {
    Full,
    Slice { start: u64, end: u64 },
    Unsatisfiable,
}

impl Range {
    pub fn parse(header: Option<&str>, size: u64) -> Self {
        let Some(header) = header else {
            return Self::Full;
        };

        let Some(spec) = header.trim().strip_prefix("bytes=") else {
            return Self::Full;
        };

        if spec.contains(',') {
            return Self::Full;
        }

        let Some((first, last)) = spec.trim().split_once('-') else {
            return Self::Full;
        };

        match (first.trim(), last.trim()) {
            ("", suffix) => match suffix.parse::<u64>() {
                Ok(0) => Self::Unsatisfiable,
                Ok(length) if size > 0 => Self::Slice {
                    start: size.saturating_sub(length),
                    end: size - 1,
                },
                Ok(_) => Self::Unsatisfiable,
                Err(_) => Self::Full,
            },
            (start, "") => match start.parse::<u64>() {
                Ok(start) if start < size => Self::Slice {
                    start,
                    end: size - 1,
                },
                Ok(_) => Self::Unsatisfiable,
                Err(_) => Self::Full,
            },
            (start, end) => match (start.parse::<u64>(), end.parse::<u64>()) {
                (Ok(start), Ok(end)) if start < size && start <= end => Self::Slice {
                    start,
                    end: end.min(size - 1),
                },
                (Ok(_), Ok(_)) => Self::Unsatisfiable,
                _ => Self::Full,
            },
        }
    }

    pub fn length(&self, size: u64) -> u64 {
        match self {
            Self::Slice { start, end } => end - start + 1,
            _ => size,
        }
    }
}

#[cfg(test)]
mod tests;
