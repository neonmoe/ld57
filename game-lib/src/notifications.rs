use engine::{allocators::LinearAllocator, collections::FixedVec};

#[derive(Clone, Copy, Debug)]
pub struct NotificationId(u32);

pub struct NotificationSet<'a, T> {
    notifications: FixedVec<'a, (u32, T)>,
    id_counter: u32,
}

impl<T> NotificationSet<'_, T> {
    pub fn new<'a>(arena: &'a LinearAllocator, capacity: usize) -> Option<NotificationSet<'a, T>> {
        Some(NotificationSet {
            notifications: FixedVec::new(arena, capacity)?,
            id_counter: 0,
        })
    }

    pub fn notify(&mut self, data: T) -> Result<NotificationId, T> {
        let id = self.id_counter;
        self.notifications
            .push((id, data))
            .map_err(|(_, data)| data)?;
        self.id_counter += 1;
        Ok(NotificationId(id))
    }

    pub fn check(&self, id: NotificationId) -> bool {
        for (id_, _) in &*self.notifications {
            if id.0 == *id_ {
                return true;
            }
        }
        false
    }

    pub fn iter(&self) -> impl Iterator<Item = (NotificationId, &T)> {
        self.notifications
            .iter()
            .map(|(id, t)| (NotificationId(*id), t))
    }

    pub fn remove(&mut self, id: NotificationId) -> Option<T> {
        let index = self
            .notifications
            .iter()
            .position(|(id_, _)| *id_ == id.0)?;
        let last_index = self.notifications.len() - 1;
        self.notifications.swap(index, last_index);
        let (_, t) = self.notifications.pop().unwrap();
        Some(t)
    }

    pub fn get_mut(&mut self, id: NotificationId) -> Option<&mut T> {
        let index = self
            .notifications
            .iter()
            .position(|(id_, _)| *id_ == id.0)?;
        Some(&mut self.notifications[index].1)
    }

    pub fn len(&self) -> usize {
        self.notifications.len()
    }
}
