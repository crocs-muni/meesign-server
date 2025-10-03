const MAX_USERNAME_LENGTH: usize = 64;

pub trait NameValidator {
    fn is_name_valid(&self) -> bool;
}

impl NameValidator for &str {
    fn is_name_valid(&self) -> bool {
        self.chars().count() <= MAX_USERNAME_LENGTH
            && !self
                .chars()
                .any(|x| x.is_ascii_punctuation() || x.is_control())
            && !self.is_empty()
    }
}

impl NameValidator for String {
    fn is_name_valid(&self) -> bool {
        self.as_str().is_name_valid()
    }
}
